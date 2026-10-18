import pathlib
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
ISO_BUILDER = ROOT / "scripts" / "make_iso.py"
SECTOR_SIZE = 2048
FAT_BOOT_IMAGE_SIZE = 1_474_560


def find_record(image: bytes, sector: int, name: bytes) -> tuple[int, int]:
    start = sector * SECTOR_SIZE
    position = start
    end = start + SECTOR_SIZE
    while position < end and image[position] != 0:
        length = image[position]
        record = image[position : position + length]
        identifier_length = record[32]
        identifier = record[33 : 33 + identifier_length]
        if identifier == name:
            return (
                struct.unpack_from("<I", record, 2)[0],
                struct.unpack_from("<I", record, 10)[0],
            )
        position += length
        if position % SECTOR_SIZE == 0:
            break
    raise AssertionError(f"ISO record {name!r} missing from sector {sector}")


class IsoBuilderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.mkdtemp(prefix="text-system-iso-test-")
        self.temporary_path = pathlib.Path(self.temporary_directory)
        self.efi_application = self.temporary_path / "TEXTOS.EFI"
        self.efi_application.write_bytes(b"original-kernel-payload")
        self.output_iso = self.temporary_path / "TEXTOS.ISO"
        self.run_builder()
        self.image = self.output_iso.read_bytes()

    def write_larger_efi(self, size: int) -> bytes:
        payload = bytes((index * 37 + 13) % 256 for index in range(size))
        self.efi_application.write_bytes(payload)
        return payload

    def tearDown(self) -> None:
        shutil.rmtree(self.temporary_directory)

    def run_builder(self, *additional_arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(ISO_BUILDER),
                "--efi",
                str(self.efi_application),
                "--out",
                str(self.output_iso),
                *additional_arguments,
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_creates_a_valid_el_torito_efi_boot_catalog(self) -> None:
        self.assertEqual(self.image[16 * SECTOR_SIZE : 17 * SECTOR_SIZE][0], 1)
        self.assertEqual(self.image[17 * SECTOR_SIZE + 1 : 17 * SECTOR_SIZE + 6], b"CD001")
        self.assertEqual(self.image[17 * SECTOR_SIZE + 88 : 17 * SECTOR_SIZE + 120], b"\0" * 32)
        self.assertEqual(self.image[18 * SECTOR_SIZE], 255)

        boot_record = self.image[SECTOR_SIZE * 17 : SECTOR_SIZE * 18]
        self.assertEqual(boot_record[7:39].rstrip(b" "), b"EL TORITO SPECIFICATION")
        catalog_sector = struct.unpack_from("<I", boot_record, 71)[0]
        catalog = self.image[catalog_sector * SECTOR_SIZE : (catalog_sector + 1) * SECTOR_SIZE]
        self.assertEqual(catalog[0], 1)
        self.assertEqual(catalog[1], 0xEF)
        validation_entry = list(struct.unpack_from("<16H", catalog, 0))
        validation_checksum = validation_entry[14]
        validation_entry[14] = 0
        self.assertEqual(
            validation_checksum,
            (-sum(validation_entry)) & 0xFFFF,
        )
        self.assertEqual(catalog[32], 0x88)
        self.assertEqual(catalog[33], 0)
        self.assertEqual(catalog[34], 0)
        self.assertEqual(catalog[35], 0)
        self.assertEqual(catalog[38:40], struct.pack("<H", FAT_BOOT_IMAGE_SIZE // 512))
        self.assertEqual(catalog[30:32], b"\x55\xaa")
        self.assertEqual(sum(struct.unpack_from("<16H", catalog)) % 65536, 0)
        self.assertEqual(len(self.image) % SECTOR_SIZE, 0)
        self.assertEqual(self.image[16 * SECTOR_SIZE + 2047], 1)

    def test_efi_system_partition_contains_a_bootable_fat12_application(self) -> None:
        boot_record = self.image[SECTOR_SIZE * 17 : SECTOR_SIZE * 18]
        catalog_sector = struct.unpack_from("<I", boot_record, 71)[0]
        catalog = self.image[catalog_sector * SECTOR_SIZE : (catalog_sector + 1) * SECTOR_SIZE]
        fat_sector = struct.unpack_from("<I", catalog, 40)[0]
        partition = self.image[fat_sector * SECTOR_SIZE :][:FAT_BOOT_IMAGE_SIZE]

        self.assertEqual(len(partition), FAT_BOOT_IMAGE_SIZE)
        self.assertEqual(partition[510:512], b"\x55\xaa")
        self.assertEqual(partition[54:62], b"FAT12   ")
        self.assertEqual(partition[43:54].rstrip(b" "), b"TEXTOS")

        fat_start = struct.unpack_from("<H", partition, 14)[0]
        root_start = (
            fat_start
            + struct.unpack_from("<H", partition, 22)[0]
            * partition[16]
        )
        root_entries = struct.unpack_from("<H", partition, 17)[0]
        root = partition[root_start * SECTOR_SIZE :][: root_entries * 32]
        directories = {}
        for offset in range(0, len(root), 32):
            entry = root[offset : offset + 32]
            if len(entry) < 32 or entry[0] == 0:
                break
            if entry[0] != 0xE5:
                directories[entry[:11]] = struct.unpack_from("<H", entry, 26)[0]
        efi_cluster = directories.get(b"EFI        ")
        self.assertIsNotNone(efi_cluster)

        data_start = root_start + (
            root_entries * 32 + SECTOR_SIZE - 1
        ) // SECTOR_SIZE
        cluster_size = partition[13] * SECTOR_SIZE
        efi_sector = data_start + (efi_cluster - 2) * partition[13]
        efi_directory = partition[efi_sector * SECTOR_SIZE :][:cluster_size]
        boot_cluster = None
        for offset in range(0, len(efi_directory), 32):
            entry = efi_directory[offset : offset + 32]
            if entry[:11] == b"BOOT       ":
                boot_cluster = struct.unpack_from("<H", entry, 26)[0]
                break
        self.assertIsNotNone(boot_cluster)

        boot_sector = data_start + (boot_cluster - 2) * partition[13]
        boot_directory = partition[boot_sector * SECTOR_SIZE :][:cluster_size]
        kernel_cluster = None
        kernel_length = None
        for offset in range(0, len(boot_directory), 32):
            entry = boot_directory[offset : offset + 32]
            if entry[:11] == b"BOOTX64 EFI":
                kernel_cluster = struct.unpack_from("<H", entry, 26)[0]
                kernel_length = struct.unpack_from("<I", entry, 28)[0]
                break
        self.assertIsNotNone(kernel_cluster)
        self.assertEqual(kernel_length, len(b"original-kernel-payload"))
        kernel_sector = data_start + (kernel_cluster - 2) * partition[13]
        self.assertEqual(
            partition[kernel_sector * SECTOR_SIZE :][:kernel_length],
            b"original-kernel-payload",
        )

    def test_iso_contains_standard_iso9660_directories_and_kernel_file(self) -> None:
        root_sector = struct.unpack_from("<I", self.image, 16 * SECTOR_SIZE + 158)[0]
        root_bytes = struct.unpack_from("<I", self.image, 16 * SECTOR_SIZE + 166)[0]
        self.assertEqual(root_bytes, SECTOR_SIZE)
        efi_sector, _ = find_record(self.image, root_sector, b"EFI")
        boot_sector, _ = find_record(self.image, efi_sector, b"BOOT")
        kernel_sector, kernel_bytes = find_record(self.image, boot_sector, b"BOOTX64.EFI;1")
        kernel_payload = self.image[
            kernel_sector * SECTOR_SIZE : kernel_sector * SECTOR_SIZE + kernel_bytes
        ]
        self.assertEqual(kernel_payload, b"original-kernel-payload")

    def test_multi_cluster_efi_image_is_preserved_in_fat12_and_iso9660(self) -> None:
        payload = self.write_larger_efi(SECTOR_SIZE * 5 + 139)
        result = self.run_builder()
        self.assertEqual(result.returncode, 0, result.stderr)
        image = self.output_iso.read_bytes()

        boot_record = image[17 * SECTOR_SIZE : 18 * SECTOR_SIZE]
        catalog_sector = struct.unpack_from("<I", boot_record, 71)[0]
        catalog = image[catalog_sector * SECTOR_SIZE : (catalog_sector + 1) * SECTOR_SIZE]
        fat_sector = struct.unpack_from("<I", catalog, 40)[0]
        partition = image[fat_sector * SECTOR_SIZE :][:FAT_BOOT_IMAGE_SIZE]
        fat_start = struct.unpack_from("<H", partition, 14)[0]
        sectors_per_fat = struct.unpack_from("<H", partition, 22)[0]
        root_entries = struct.unpack_from("<H", partition, 17)[0]
        root_start = fat_start + 2 * sectors_per_fat
        data_start = root_start + (root_entries * 32 + SECTOR_SIZE - 1) // SECTOR_SIZE

        def find_fat_file(sector: int, expected_name: bytes) -> tuple[int, int]:
            data = partition[sector * SECTOR_SIZE : (sector + 1) * SECTOR_SIZE]
            for offset in range(0, len(data), 32):
                entry = data[offset : offset + 32]
                if entry[:11] == expected_name:
                    return struct.unpack_from("<H", entry, 26)[0], struct.unpack_from(
                        "<I", entry, 28
                    )[0]
            raise AssertionError(f"FAT entry {expected_name!r} missing from sector {sector}")

        efi_cluster, _ = find_fat_file(root_start, b"EFI        ")
        efi_sector = data_start + efi_cluster - 2
        boot_cluster, _ = find_fat_file(efi_sector, b"BOOT       ")
        boot_sector = data_start + boot_cluster - 2
        kernel_cluster, kernel_size = find_fat_file(boot_sector, b"BOOTX64 EFI")
        self.assertEqual(kernel_size, len(payload))

        fat = partition[fat_start * SECTOR_SIZE :][
            : sectors_per_fat * SECTOR_SIZE
        ]
        remaining = len(payload)
        cluster = kernel_cluster
        actual_fat_payload = bytearray()
        while remaining:
            cluster_offset = data_start + cluster - 2
            chunk = min(remaining, SECTOR_SIZE)
            actual_fat_payload.extend(
                partition[cluster_offset * SECTOR_SIZE :][:chunk]
            )
            remaining -= chunk
            pair_offset = cluster + cluster // 2
            encoded = fat[pair_offset] | (fat[pair_offset + 1] << 8)
            next_cluster = (encoded >> 4 if cluster & 1 else encoded) & 0xFFF
            if remaining:
                self.assertNotIn(next_cluster, (0, 0xFF7, 0xFF8, 0xFF9, 0xFFA, 0xFFB, 0xFFC, 0xFFD, 0xFFE, 0xFFF))
            else:
                self.assertGreaterEqual(next_cluster, 0xFF8)
            cluster = next_cluster
        self.assertEqual(actual_fat_payload, payload)

        root_iso_sector = struct.unpack_from("<I", image, 16 * SECTOR_SIZE + 158)[0]
        efi_iso_sector, _ = find_record(image, root_iso_sector, b"EFI")
        iso_boot_sector, _ = find_record(image, efi_iso_sector, b"BOOT")
        kernel_iso_sector, kernel_iso_size = find_record(
            image, iso_boot_sector, b"BOOTX64.EFI;1"
        )
        self.assertEqual(kernel_iso_size, len(payload))
        self.assertEqual(
            image[
                kernel_iso_sector * SECTOR_SIZE : kernel_iso_sector * SECTOR_SIZE
                + kernel_iso_size
            ],
            payload,
        )

    def test_refuses_to_overwrite_the_input_kernel(self) -> None:
        self.assertFalse(self.efi_application.resolve() == self.output_iso.resolve())
        result = self.run_builder("--out", str(self.efi_application))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("input EFI application", result.stderr)
        self.assertEqual(self.efi_application.read_bytes(), b"original-kernel-payload")

    def test_refuses_an_unsupported_efi_application_name(self) -> None:
        result = self.run_builder("--name", "../outside")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("volume name", result.stderr)

    def test_rejects_empty_efi_application_without_creating_iso(self) -> None:
        self.efi_application.write_bytes(b"")
        self.output_iso.unlink()
        result = self.run_builder()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("EFI application is empty", result.stderr)
        self.assertFalse(self.output_iso.exists())

    def test_failed_iso_build_preserves_existing_image(self) -> None:
        existing_image = self.output_iso.read_bytes()
        self.efi_application.write_bytes(b"")

        result = self.run_builder()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("EFI application is empty", result.stderr)
        self.assertEqual(self.output_iso.read_bytes(), existing_image)


if __name__ == "__main__":
    unittest.main()
