#!/usr/bin/env python3
"""Build a self-contained El Torito UEFI ISO using only Python's standard library."""

from __future__ import annotations

import argparse
import math
import pathlib
import re
import struct
import sys
import tempfile

SECTOR_SIZE = 2048
FAT_BOOT_IMAGE_SIZE = 1_474_560
FAT_SECTORS_PER_CLUSTER = 1
FAT_ROOT_ENTRY_COUNT = 224
FAT_SIZE_SECTORS = 9
EFI_SYSTEM_PARTITION_TYPE = 0xEF
ISO_APPLICATION_IDENTIFIER = b"ORIGINAL TEXT SYSTEM"
INPUT_STABILITY_RETRIES = 3


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def fat12_set_entry(fat: bytearray, cluster: int, value: int) -> None:
    offset = cluster + cluster // 2
    if cluster & 1:
        fat[offset] = (fat[offset] & 0x0F) | ((value << 4) & 0xF0)
        fat[offset + 1] = (value >> 4) & 0xFF
    else:
        fat[offset] = value & 0xFF
        fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) & 0x0F)


def fat_directory_entry(
    filename: bytes, attributes: int, first_cluster: int, size: int
) -> bytes:
    require(len(filename) == 11, "FAT directory names must contain exactly 11 bytes")
    entry = bytearray(32)
    entry[:11] = filename
    entry[11] = attributes
    struct.pack_into("<H", entry, 26, first_cluster)
    struct.pack_into("<I", entry, 28, size)
    return bytes(entry)


def build_fat_esp(efi: bytes, volume_label: str) -> bytes:
    require(
        len(efi) > 0,
        "input EFI application is empty",
    )

    sectors_per_fat = FAT_SIZE_SECTORS
    reserved_sectors = 1
    fat_count = 2
    root_sectors = math.ceil(FAT_ROOT_ENTRY_COUNT * 32 / SECTOR_SIZE)
    data_start = reserved_sectors + fat_count * sectors_per_fat + root_sectors
    cluster_size = FAT_SECTORS_PER_CLUSTER * SECTOR_SIZE

    kernel_cluster_count = math.ceil(len(efi) / cluster_size)
    efi_directory_cluster = 2
    boot_directory_cluster = 3
    kernel_first_cluster = 4
    last_kernel_cluster = kernel_first_cluster + kernel_cluster_count - 1
    data_cluster_count = last_kernel_cluster - 1

    require(
        last_kernel_cluster <= 0xFEF,
        "EFI application is too large to fit on the FAT12 UEFI boot image",
    )
    require(
        data_start + data_cluster_count * FAT_SECTORS_PER_CLUSTER
        <= FAT_BOOT_IMAGE_SIZE // SECTOR_SIZE,
        "generated FAT12 filesystem exceeds its boot-image boundary",
    )

    image = bytearray(FAT_BOOT_IMAGE_SIZE)
    boot_sector = memoryview(image)[:SECTOR_SIZE]
    boot_sector[0:3] = b"\xEB\x3C\x90"
    boot_sector[3:11] = b"TEXTOSYS"
    struct.pack_into("<H", boot_sector, 11, SECTOR_SIZE)
    boot_sector[13] = FAT_SECTORS_PER_CLUSTER
    struct.pack_into("<H", boot_sector, 14, reserved_sectors)
    boot_sector[16] = fat_count
    struct.pack_into("<H", boot_sector, 17, FAT_ROOT_ENTRY_COUNT)
    struct.pack_into("<H", boot_sector, 19, FAT_BOOT_IMAGE_SIZE // SECTOR_SIZE)
    boot_sector[21] = 0xF0
    struct.pack_into("<H", boot_sector, 22, sectors_per_fat)
    struct.pack_into("<H", boot_sector, 24, 18)
    struct.pack_into("<H", boot_sector, 26, 2)
    boot_sector[36] = 0
    boot_sector[38] = 0x29
    struct.pack_into("<I", boot_sector, 39, 0x5458_4F53)
    boot_sector[43:54] = volume_label.encode("ascii").ljust(11, b" ")
    boot_sector[54:62] = b"FAT12   "
    boot_sector[510:512] = b"\x55\xAA"

    fat_bytes = sectors_per_fat * SECTOR_SIZE
    fat = bytearray(fat_bytes)
    fat[:3] = b"\xF0\xFF\xFF"

    def occupy(cluster: int, following: int) -> None:
        fat12_set_entry(fat, cluster, following)

    occupy(efi_directory_cluster, 0xFFF)
    occupy(boot_directory_cluster, 0xFFF)
    for cluster in range(kernel_first_cluster, last_kernel_cluster):
        occupy(cluster, cluster + 1)
    occupy(last_kernel_cluster, 0xFFF)

    for copy in range(fat_count):
        start_sector = reserved_sectors + copy * sectors_per_fat
        start = start_sector * SECTOR_SIZE
        image[start : start + fat_bytes] = fat

    root_sector = reserved_sectors + fat_count * sectors_per_fat
    root_start = root_sector * SECTOR_SIZE
    image[root_start : root_start + 32] = fat_directory_entry(
        b"EFI        ", 0x10, efi_directory_cluster, 0
    )
    image[root_start + 32 : root_start + 64] = fat_directory_entry(
        b"TEXTOSYS   ", 0x08, 0, 0
    )

    def data_cluster_offset(cluster: int) -> int:
        return (data_start + (cluster - 2) * FAT_SECTORS_PER_CLUSTER) * SECTOR_SIZE

    efi_offset = data_cluster_offset(efi_directory_cluster)
    image[efi_offset : efi_offset + 32] = fat_directory_entry(
        b".          ", 0x10, efi_directory_cluster, 0
    )
    image[efi_offset + 32 : efi_offset + 64] = fat_directory_entry(
        b"..         ", 0x10, 0, 0
    )
    image[efi_offset + 64 : efi_offset + 96] = fat_directory_entry(
        b"BOOT       ", 0x10, boot_directory_cluster, 0
    )

    boot_offset = data_cluster_offset(boot_directory_cluster)
    image[boot_offset : boot_offset + 32] = fat_directory_entry(
        b".          ", 0x10, boot_directory_cluster, 0
    )
    image[boot_offset + 32 : boot_offset + 64] = fat_directory_entry(
        b"..         ", 0x10, efi_directory_cluster, 0
    )
    image[boot_offset + 64 : boot_offset + 96] = fat_directory_entry(
        b"BOOTX64 EFI", 0x20, kernel_first_cluster, len(efi)
    )

    kernel_offset = data_cluster_offset(kernel_first_cluster)
    image[kernel_offset : kernel_offset + len(efi)] = efi
    return bytes(image)


def iso_directory_record(
    name: bytes,
    extent_sector: int,
    size_bytes: int,
    *,
    directory: bool = False,
) -> bytes:
    require(len(name) <= 255, "ISO9660 directory record name is too long")
    record_length = 33 + len(name) + (1 if len(name) % 2 == 0 else 0)
    record = bytearray(record_length)
    record[0] = record_length
    record[1] = 0
    struct.pack_into("<I", record, 2, extent_sector)
    struct.pack_into(">I", record, 6, extent_sector)
    struct.pack_into("<I", record, 10, size_bytes)
    struct.pack_into(">I", record, 14, size_bytes)
    record[18:25] = bytes((126, 1, 1, 0, 0, 0, 0))
    record[25] = 0x02 if directory else 0
    record[28] = 1
    record[31] = 1
    record[32] = len(name)
    record[33 : 33 + len(name)] = name
    return bytes(record)


def pack_iso_directory(entries: list[bytes]) -> bytes:
    sector = bytearray(SECTOR_SIZE)
    cursor = 0
    for entry in entries:
        if cursor + len(entry) > SECTOR_SIZE:
            raise ValueError("ISO9660 directory entries exceed one sector")
        sector[cursor : cursor + len(entry)] = entry
        cursor += len(entry)
    return bytes(sector)


def build_iso(efi: bytes, volume_label: str) -> bytes:
    partition = build_fat_esp(efi, volume_label)
    partition_sectors = len(partition) // SECTOR_SIZE

    pvd_sector = 16
    boot_record_sector = 17
    terminator_sector = 18
    catalog_sector = 19
    partition_sector = 20
    root_sector = 20 + partition_sectors
    efi_sector = root_sector + 1
    boot_sector = efi_sector + 1
    kernel_sector = boot_sector + 1

    kernel_record = iso_directory_record(
        b"BOOTX64.EFI;1", kernel_sector, len(efi)
    )
    boot_directory = pack_iso_directory(
        [
            iso_directory_record(b"\x00", boot_sector, SECTOR_SIZE, directory=True),
            iso_directory_record(b"\x01", efi_sector, SECTOR_SIZE, directory=True),
            kernel_record,
        ]
    )
    efi_directory = pack_iso_directory(
        [
            iso_directory_record(b"\x00", efi_sector, SECTOR_SIZE, directory=True),
            iso_directory_record(b"\x01", root_sector, SECTOR_SIZE, directory=True),
            iso_directory_record(b"BOOT", boot_sector, SECTOR_SIZE, directory=True),
        ]
    )
    root_directory = pack_iso_directory(
        [
            iso_directory_record(b"\x00", root_sector, SECTOR_SIZE, directory=True),
            iso_directory_record(b"\x01", root_sector, SECTOR_SIZE, directory=True),
            iso_directory_record(b"EFI", efi_sector, SECTOR_SIZE, directory=True),
        ]
    )

    sectors = [bytearray(SECTOR_SIZE) for _ in range(kernel_sector + 1)]

    pvd = sectors[pvd_sector]
    pvd[0] = 1
    pvd[1:6] = b"CD001"
    pvd[6] = 1
    pvd[8:40] = b"TEXT SYSTEM".ljust(32, b" ")
    pvd[40:72] = volume_label.encode("ascii").ljust(32, b" ")
    struct.pack_into("<I", pvd, 80, len(sectors))
    struct.pack_into(">I", pvd, 84, len(sectors))
    struct.pack_into("<H", pvd, 120, 1)
    struct.pack_into(">H", pvd, 122, 1)
    struct.pack_into("<H", pvd, 124, 1)
    struct.pack_into(">H", pvd, 126, 1)
    struct.pack_into("<H", pvd, 128, SECTOR_SIZE)
    struct.pack_into(">H", pvd, 130, SECTOR_SIZE)
    root_record = iso_directory_record(
        b"\x00", root_sector, SECTOR_SIZE, directory=True
    )
    pvd[156 : 156 + len(root_record)] = root_record
    pvd[881] = 1
    pvd[882:1395] = bytes(513)
    pvd[813:830] = b"2026101800000000\x00"
    pvd[830:847] = b"2026101800000000\x00"
    pvd[847:864] = bytes(17)
    pvd[864:881] = bytes(17)
    pvd[1423] = 1
    pvd[1424] = 0
    pvd[1425] = 0
    pvd[1426] = 1
    pvd[574:702] = ISO_APPLICATION_IDENTIFIER.ljust(128, b" ")
    pvd[2047] = 1

    boot_record = sectors[boot_record_sector]
    boot_record[0] = 0
    boot_record[1:6] = b"CD001"
    boot_record[6] = 1
    boot_record[7:39] = b"EL TORITO SPECIFICATION".ljust(32, b" ")
    struct.pack_into("<I", boot_record, 71, catalog_sector)

    terminator = sectors[terminator_sector]
    terminator[0] = 255
    terminator[1:6] = b"CD001"
    terminator[6] = 1

    catalog = sectors[catalog_sector]
    catalog[0] = 1
    catalog[1] = EFI_SYSTEM_PARTITION_TYPE
    catalog[2] = 0
    catalog[4] = 0
    catalog[30:32] = b"\x55\xaa"
    words = [0xEF01] + [0] * 14 + [0xAA55]
    words[14] = (-sum(words)) & 0xFFFF
    struct.pack_into("<16H", catalog, 0, *words)
    catalog[32] = 0x88
    catalog[33] = 0
    catalog[34] = 0
    catalog[35] = 0
    struct.pack_into("<H", catalog, 38, FAT_BOOT_IMAGE_SIZE // 512)
    struct.pack_into("<I", catalog, 40, partition_sector)

    sectors[root_sector][:] = root_directory
    sectors[efi_sector][:] = efi_directory
    sectors[boot_sector][:] = boot_directory
    sectors[kernel_sector][: len(efi)] = efi

    result = bytearray().join(sectors)
    partition_start = partition_sector * SECTOR_SIZE
    result[partition_start : partition_start + len(partition)] = partition
    return bytes(result)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--efi", type=pathlib.Path, required=True, help="compiled EFI kernel")
    parser.add_argument("--out", type=pathlib.Path, required=True, help="output ISO9660 file")
    parser.add_argument("--name", default="TEXTOS", help="FAT/ISO volume name (1-11 A-Z, 0-9)")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    arguments = parse_args(argv)
    try:
        volume_label = arguments.name.upper()
        require(
            bool(re.fullmatch(r"[A-Z0-9_-]{1,11}", volume_label)),
            "volume name must be 1-11 ASCII letters, digits, underscores, or hyphens",
        )
        efi_path = arguments.efi.resolve(strict=True)
        require(efi_path.is_file(), "input EFI application must be a file")
        out_path = arguments.out.resolve()
        require(efi_path != out_path, "output cannot replace the input EFI application")
        image = None
        for attempt in range(INPUT_STABILITY_RETRIES):
            before = efi_path.stat()
            efi_contents = efi_path.read_bytes()
            after = efi_path.stat()
            is_stable = (
                before.st_ino == after.st_ino
                and before.st_size == after.st_size == len(efi_contents)
                and before.st_mtime_ns == after.st_mtime_ns
            )
            if is_stable:
                require(efi_contents, "input EFI application is empty")
                image = build_iso(efi_contents, volume_label)
                break
            if attempt + 1 == INPUT_STABILITY_RETRIES:
                raise ValueError("input EFI application changed while it was being read")
        if image is None:
            raise ValueError("input EFI application could not be read consistently")
        out_path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(
            dir=out_path.parent,
            prefix=f".{out_path.name}.",
            suffix=".tmp",
            delete=False,
        ) as temporary_file:
            temporary_path = pathlib.Path(temporary_file.name)
            temporary_file.write(image)
            temporary_file.flush()
        try:
            temporary_path.replace(out_path)
        except BaseException:
            temporary_path.unlink(missing_ok=True)
            raise
    except (OSError, ValueError) as error:
        print(f"ISO build failed: {error}", file=sys.stderr)
        return 1

    print(f"Created UEFI-bootable ISO: {out_path} ({len(image)} bytes)")
    print("The ISO includes an El Torito EFI boot entry and a FAT12 EFI system partition.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
