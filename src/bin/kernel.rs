#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;
use text_system::memory::BuddyAllocator;

const EFI_SUCCESS: usize = 0;
const EFI_CONVENTIONAL_MEMORY: u32 = 7;
const MEMORY_MAP_BYTES: usize = 64 * 1024;
const GOP_RGB_RESERVED: u32 = 0;
const GOP_BGR_RESERVED: u32 = 1;

static mut MEMORY_MAP: [u64; MEMORY_MAP_BYTES / size_of::<u64>()] =
    [0; MEMORY_MAP_BYTES / size_of::<u64>()];
static mut KERNEL_ALLOCATOR: BuddyAllocator = BuddyAllocator::new();

#[derive(Clone, Copy)]
#[repr(C)]
struct TableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    header: TableHeader,
    firmware_vendor: *const u16,
    firmware_revision: u32,
    _alignment: u32,
    console_input_handle: Handle,
    console_input: *mut c_void,
    console_output_handle: Handle,
    console_output: *mut SimpleTextOutput,
    standard_error_handle: Handle,
    standard_error: *mut c_void,
    runtime_services: *mut c_void,
    boot_services: *mut BootServices,
    configuration_table_count: usize,
    configuration_table: *mut c_void,
}

type Handle = *mut c_void;
type OutputString = unsafe extern "efiapi" fn(*mut SimpleTextOutput, *const u16) -> usize;

#[repr(C)]
struct SimpleTextOutput {
    reset: usize,
    output_string: OutputString,
}

type GetMemoryMap =
    unsafe extern "efiapi" fn(*mut usize, *mut u8, *mut usize, *mut usize, *mut u32) -> usize;
type ExitBootServices = unsafe extern "efiapi" fn(Handle, usize) -> usize;
type LocateProtocol = unsafe extern "efiapi" fn(*const Guid, Handle, *mut *mut c_void) -> usize;

#[repr(C)]
struct BootServices {
    header: TableHeader,
    initial_functions: [usize; 4],
    get_memory_map: GetMemoryMap,
    functions_before_exit: [usize; 21],
    exit_boot_services: ExitBootServices,
    functions_before_locate_protocol: [usize; 10],
    locate_protocol: LocateProtocol,
}

const _: () = {
    assert!(size_of::<usize>() == 8);
    assert!(core::mem::offset_of!(SystemTable, boot_services) == 96);
    assert!(core::mem::offset_of!(BootServices, get_memory_map) == 56);
    assert!(core::mem::offset_of!(BootServices, exit_boot_services) == 232);
    assert!(core::mem::offset_of!(BootServices, locate_protocol) == 320);
};

#[derive(Clone, Copy)]
#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const GOP_GUID: Guid = Guid {
    data1: 0x9042_a9de,
    data2: 0x23dc,
    data3: 0x4a38,
    data4: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

#[repr(C)]
struct GraphicsOutputProtocol {
    query_mode: usize,
    set_mode: usize,
    blit: usize,
    mode: *mut GraphicsOutputMode,
}

#[repr(C)]
struct GraphicsOutputMode {
    max_mode: u32,
    mode: u32,
    info: *mut GraphicsOutputModeInfo,
    info_size: usize,
    framebuffer_base: u64,
    framebuffer_size: usize,
}

#[repr(C)]
struct GraphicsOutputModeInfo {
    version: u32,
    horizontal_resolution: u32,
    vertical_resolution: u32,
    pixel_format: u32,
    pixel_bitmask: [u32; 4],
    pixels_per_scan_line: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct MemoryDescriptor {
    memory_type: u32,
    _padding: u32,
    physical_start: u64,
    virtual_start: u64,
    page_count: u64,
    attributes: u64,
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack)) };
    }
}

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(_image_handle: Handle, system_table: *mut SystemTable) -> usize {
    if system_table.is_null() {
        return 1;
    }

    let table = unsafe { &*system_table };
    let services = unsafe { &*table.boot_services };
    print_firmware_message(table.console_output);

    let mut graphics = ptr::null_mut();
    let status = unsafe { (services.locate_protocol)(&GOP_GUID, ptr::null_mut(), &mut graphics) };
    let framebuffer = if status == EFI_SUCCESS && !graphics.is_null() {
        unsafe { capture_framebuffer(graphics.cast::<GraphicsOutputProtocol>()) }
    } else {
        None
    };

    let mut buffer_size = MEMORY_MAP_BYTES;
    let mut map_key = 0usize;
    let mut descriptor_size = 0usize;
    let mut descriptor_version = 0u32;
    let map = ptr::addr_of_mut!(MEMORY_MAP).cast::<u8>();
    let mut exit_status = 1usize;
    let mut map_ready = false;

    for _ in 0..3 {
        buffer_size = MEMORY_MAP_BYTES;
        let status = unsafe {
            (services.get_memory_map)(
                &mut buffer_size,
                map,
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            )
        };
        if status != EFI_SUCCESS {
            return status;
        }
        if descriptor_size < size_of::<MemoryDescriptor>() {
            return 1;
        }

        exit_status = unsafe { (services.exit_boot_services)(_image_handle, map_key) };
        if exit_status == EFI_SUCCESS {
            map_ready = true;
            break;
        }
    }
    if !map_ready {
        return exit_status;
    }

    let total_ram = unsafe { initialize_physical_allocator(map, buffer_size, descriptor_size) };
    if let Some(framebuffer) = framebuffer {
        unsafe { render_desktop(framebuffer, total_ram) };
    }

    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack)) };
    }
}

fn print_firmware_message(console: *mut SimpleTextOutput) {
    if console.is_null() {
        return;
    }
    static MESSAGE: &[u16] = &[
        13, 10, 84, 69, 88, 84, 32, 83, 89, 83, 84, 69, 77, 32, 45, 32, 73, 78, 73, 84, 73, 65, 76,
        73, 90, 73, 78, 71, 46, 46, 46, 13, 10, 0,
    ];
    let protocol = unsafe { &mut *console };
    let _ = unsafe { (protocol.output_string)(protocol, MESSAGE.as_ptr()) };
}

unsafe fn initialize_physical_allocator(
    map: *const u8,
    map_bytes: usize,
    descriptor_size: usize,
) -> u64 {
    let allocator = ptr::addr_of_mut!(KERNEL_ALLOCATOR);
    let mut total_ram = 0u64;
    let mut offset = 0usize;

    while offset
        .checked_add(size_of::<MemoryDescriptor>())
        .is_some_and(|end| end <= map_bytes)
    {
        let descriptor = unsafe { ptr::read_unaligned(map.add(offset).cast::<MemoryDescriptor>()) };
        if descriptor.memory_type == EFI_CONVENTIONAL_MEMORY
            && descriptor.page_count != 0
            && descriptor.physical_start != 0
        {
            let size_bytes = descriptor.page_count.saturating_mul(4096);
            let result =
                unsafe { (&mut *allocator).add_region(descriptor.physical_start, size_bytes) };
            if result.is_ok() {
                total_ram = total_ram.saturating_add(size_bytes);
            }
        }
        let Some(next) = offset.checked_add(descriptor_size) else {
            break;
        };
        offset = next;
    }
    total_ram
}

#[derive(Clone, Copy)]
struct Framebuffer {
    base: *mut u8,
    byte_len: usize,
    width: usize,
    height: usize,
    stride: usize,
    bgr: bool,
}

unsafe fn capture_framebuffer(graphics: *mut GraphicsOutputProtocol) -> Option<Framebuffer> {
    let protocol = unsafe { &*graphics };
    if protocol.mode.is_null() {
        return None;
    }
    let mode = unsafe { &*protocol.mode };
    if mode.info.is_null() || mode.framebuffer_base == 0 {
        return None;
    }
    let info = unsafe { &*mode.info };
    if !matches!(info.pixel_format, GOP_RGB_RESERVED | GOP_BGR_RESERVED) {
        return None;
    }

    Some(Framebuffer {
        base: mode.framebuffer_base as *mut u8,
        byte_len: mode.framebuffer_size,
        width: info.horizontal_resolution as usize,
        height: info.vertical_resolution as usize,
        stride: info.pixels_per_scan_line as usize,
        bgr: info.pixel_format == GOP_BGR_RESERVED,
    })
}

unsafe fn render_desktop(framebuffer: Framebuffer, total_ram: u64) {
    let Framebuffer {
        base,
        byte_len,
        width,
        height,
        stride,
        bgr,
    } = framebuffer;
    let screen = Screen {
        base,
        byte_len,
        width,
        height,
        stride,
        bgr,
    };
    if screen.width < 320
        || screen.height < 240
        || screen.stride < screen.width
        || screen
            .stride
            .checked_mul(screen.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .is_none_or(|bytes| bytes > screen.byte_len)
    {
        return;
    }

    screen.paint_background();
    let scale = (screen.height / 100).clamp(2, 8);
    let margin = (screen.width / 12).max(24);
    let card_x = margin;
    let card_y = screen.height / 4;
    let card_width = screen.width.saturating_sub(margin * 2).max(1);
    let card_height = (screen.height / 2).max(1);

    let card_radius = screen.width / 36;
    screen.rounded_rectangle(
        card_x,
        card_y,
        card_width,
        card_height,
        card_radius,
        [208, 221, 255],
        67,
    );
    screen.rounded_rectangle(
        card_x,
        card_y,
        card_width,
        card_radius / 4 + 1,
        (card_radius / 8).max(1),
        [189, 223, 255],
        150,
    );
    screen.rectangle(
        card_x + margin / 2,
        card_y + margin / 2,
        3,
        card_height.saturating_sub(margin),
        [151, 181, 255],
    );

    let eyebrow_scale = scale.min(3);
    screen.text(
        b"ACADEMY NETWORK  /  X64",
        card_x + margin,
        card_y + margin / 2,
        eyebrow_scale,
        [166, 195, 255],
    );

    let title_scale = fit_text_scale(b"TEXT SYSTEM", card_width / 2, scale);
    screen.text(
        b"TEXT SYSTEM",
        card_x + margin,
        card_y + margin + eyebrow_scale * 11,
        title_scale,
        [255, 255, 255],
    );

    let status_scale = scale.min(3);
    screen.text(
        b"UEFI SYSTEM READY",
        card_x + margin,
        card_y + margin + eyebrow_scale * 11 + title_scale * 11,
        status_scale,
        [177, 222, 210],
    );

    let stats_y = card_y + card_height * 2 / 3;
    let stats_scale = scale.min(2);
    screen.text(
        b"PHYSICAL MEMORY",
        card_x + margin,
        stats_y,
        stats_scale,
        [180, 194, 228],
    );

    let mut digits = [0u8; 20];
    let digit_length = format_megabytes(total_ram, &mut digits);
    screen.text(
        &digits[..digit_length],
        card_x + margin,
        stats_y + stats_scale * 10,
        scale.min(4),
        [255, 255, 255],
    );
    screen.text(
        b"MEGABYTES MANAGED",
        card_x + margin,
        stats_y + stats_scale * 10 + scale.min(4) * 11,
        stats_scale,
        [180, 194, 228],
    );

    let footer = screen.height.saturating_sub(margin);
    screen.text(
        b"BLUE HORIZON  /  ORIGINAL X64 SYSTEM",
        margin,
        footer,
        status_scale,
        [167, 182, 215],
    );

    let demonstration = unsafe { (&mut *ptr::addr_of_mut!(KERNEL_ALLOCATOR)).allocate(0) };
    if let Ok(block) = demonstration {
        let _ = unsafe { (&mut *ptr::addr_of_mut!(KERNEL_ALLOCATOR)).free(block) };
    }
}

fn fit_text_scale(text: &[u8], available_width: usize, requested: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let glyph_units = text.len().saturating_mul(6).saturating_sub(1).max(1);
    requested.min((available_width / glyph_units).max(1))
}

fn format_megabytes(bytes: u64, buffer: &mut [u8; 20]) -> usize {
    let mut value = bytes / (1024 * 1024);
    let mut cursor = buffer.len();

    if value == 0 {
        cursor -= 1;
        buffer[cursor] = b'0';
    } else {
        while value != 0 {
            cursor -= 1;
            buffer[cursor] = b'0' + (value % 10) as u8;
            value /= 10;
        }
    }

    let length = buffer.len() - cursor;
    buffer.copy_within(cursor.., 0);
    length
}

struct Screen {
    base: *mut u8,
    byte_len: usize,
    width: usize,
    height: usize,
    stride: usize,
    bgr: bool,
}

impl Screen {
    fn paint_background(&self) {
        for y in 0..self.height {
            for x in 0..self.width {
                let vertical = ((y * 22) / self.height) as i32;
                let horizontal = ((x * 18) / self.width) as i32;
                let shifted_x = x.abs_diff(self.width * 3 / 4);
                let shifted_y = y.abs_diff(self.height / 4);
                let radial_distance = shifted_x / 7 + shifted_y / 5;
                let halo = 52usize.saturating_sub(radial_distance).min(40) as i32;
                self.set_pixel(
                    x,
                    y,
                    [
                        (9 + horizontal + halo / 5) as u8,
                        (15 + vertical + halo / 2) as u8,
                        (34 + vertical + halo) as u8,
                    ],
                );
            }
        }
    }

    fn rectangle(&self, x: usize, y: usize, width: usize, height: usize, color: [u8; 3]) {
        let end_x = x.saturating_add(width).min(self.width);
        let end_y = y.saturating_add(height).min(self.height);
        for pixel_y in y.min(self.height)..end_y {
            for pixel_x in x.min(self.width)..end_x {
                self.set_pixel(pixel_x, pixel_y, color);
            }
        }
    }

    fn rounded_rectangle(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        radius: usize,
        color: [u8; 3],
        alpha: u16,
    ) {
        let start_x = x.min(self.width);
        let start_y = y.min(self.height);
        let end_x = x.saturating_add(width).min(self.width);
        let end_y = y.saturating_add(height).min(self.height);
        let radius = radius.min(width / 2).min(height / 2) as i64;

        for pixel_y in start_y..end_y {
            for pixel_x in start_x..end_x {
                let local_x = (pixel_x - x) as i64;
                let local_y = (pixel_y - y) as i64;
                let closest_x = local_x.clamp(radius, width as i64 - radius - 1);
                let closest_y = local_y.clamp(radius, height as i64 - radius - 1);
                let dx = local_x - closest_x;
                let dy = local_y - closest_y;
                if dx * dx + dy * dy <= radius * radius {
                    self.blend_pixel(pixel_x, pixel_y, color, alpha);
                }
            }
        }
    }

    fn text(&self, text: &[u8], x: usize, y: usize, scale: usize, color: [u8; 3]) {
        let scale = scale.max(1);
        let mut cursor_x = x;
        for &character in text {
            if character == b' ' {
                cursor_x = cursor_x.saturating_add(scale * 4);
                continue;
            }
            if let Some(glyph) = glyph(character) {
                for (column, bits) in glyph.iter().enumerate() {
                    for row in 0..7 {
                        if bits & (1 << row) != 0 {
                            self.rectangle(
                                cursor_x.saturating_add(column * scale),
                                y.saturating_add(row * scale),
                                scale,
                                scale,
                                color,
                            );
                        }
                    }
                }
            }
            cursor_x = cursor_x.saturating_add(scale * 6);
        }
    }

    fn blend_pixel(&self, x: usize, y: usize, color: [u8; 3], alpha: u16) {
        let Some(offset) = self.pixel_offset(x, y) else {
            return;
        };
        let mut channels = unsafe {
            [
                self.base.add(offset).read_volatile(),
                self.base.add(offset + 1).read_volatile(),
                self.base.add(offset + 2).read_volatile(),
            ]
        };
        if self.bgr {
            channels.swap(0, 2);
        }
        let mut output = [0; 3];
        for channel in 0..3 {
            output[channel] = ((u16::from(channels[channel]) * (255 - alpha)
                + u16::from(color[channel]) * alpha)
                / 255) as u8;
        }
        self.write_pixel(offset, output);
    }

    fn set_pixel(&self, x: usize, y: usize, color: [u8; 3]) {
        let Some(offset) = self.pixel_offset(x, y) else {
            return;
        };
        self.write_pixel(offset, color);
    }

    fn pixel_offset(&self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = y.checked_mul(self.stride)?.checked_add(x)?.checked_mul(4)?;
        (offset.checked_add(3)? < self.byte_len).then_some(offset)
    }

    fn write_pixel(&self, offset: usize, mut color: [u8; 3]) {
        if self.bgr {
            color.swap(0, 2);
        }
        unsafe {
            self.base.add(offset).write_volatile(color[0]);
            self.base.add(offset + 1).write_volatile(color[1]);
            self.base.add(offset + 2).write_volatile(color[2]);
            self.base.add(offset + 3).write_volatile(0);
        }
    }
}

fn glyph(character: u8) -> Option<[u8; 5]> {
    let columns = match character {
        b'-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        b'A' => [0x7e, 0x11, 0x11, 0x11, 0x7e],
        b'B' => [0x7f, 0x49, 0x49, 0x49, 0x36],
        b'C' => [0x3e, 0x41, 0x41, 0x41, 0x22],
        b'D' => [0x7f, 0x41, 0x41, 0x22, 0x1c],
        b'E' => [0x7f, 0x49, 0x49, 0x49, 0x41],
        b'F' => [0x7f, 0x09, 0x09, 0x09, 0x01],
        b'G' => [0x3e, 0x41, 0x49, 0x49, 0x7a],
        b'H' => [0x7f, 0x08, 0x08, 0x08, 0x7f],
        b'I' => [0x00, 0x41, 0x7f, 0x41, 0x00],
        b'J' => [0x20, 0x40, 0x41, 0x3f, 0x01],
        b'K' => [0x7f, 0x08, 0x14, 0x22, 0x41],
        b'L' => [0x7f, 0x40, 0x40, 0x40, 0x40],
        b'M' => [0x7f, 0x02, 0x0c, 0x02, 0x7f],
        b'N' => [0x7f, 0x04, 0x08, 0x10, 0x7f],
        b'O' => [0x3e, 0x41, 0x41, 0x41, 0x3e],
        b'P' => [0x7f, 0x09, 0x09, 0x09, 0x06],
        b'Q' => [0x3e, 0x41, 0x51, 0x21, 0x5e],
        b'R' => [0x7f, 0x09, 0x19, 0x29, 0x46],
        b'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        b'T' => [0x01, 0x01, 0x7f, 0x01, 0x01],
        b'U' => [0x3f, 0x40, 0x40, 0x40, 0x3f],
        b'V' => [0x1f, 0x20, 0x40, 0x20, 0x1f],
        b'W' => [0x7f, 0x20, 0x18, 0x20, 0x7f],
        b'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        b'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
        b'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        b'0' => [0x3e, 0x51, 0x49, 0x45, 0x3e],
        b'1' => [0x00, 0x42, 0x7f, 0x40, 0x00],
        b'2' => [0x62, 0x51, 0x49, 0x49, 0x46],
        b'3' => [0x22, 0x41, 0x49, 0x49, 0x36],
        b'4' => [0x18, 0x14, 0x12, 0x7f, 0x10],
        b'5' => [0x2f, 0x49, 0x49, 0x49, 0x31],
        b'6' => [0x3e, 0x49, 0x49, 0x49, 0x32],
        b'7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        b'8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        b'9' => [0x26, 0x49, 0x49, 0x49, 0x3e],
        b'/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        _ => return None,
    };
    Some(columns)
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
