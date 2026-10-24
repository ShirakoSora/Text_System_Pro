# Text System

Text System is an original, dependency-free Rust x86_64 UEFI kernel experiment with an original, cheerful blue-and-white academy/sci-fi interface, cyan indicators, magenta accents, and a translucent dashboard. The design does not reproduce Blue Archive character art, logos, or game assets. The EFI application is the kernel: the firmware enters it using the native UEFI ABI, so this first firmware-booted stage does not need a second hand-written reset-vector or legacy BIOS assembly bootloader. The kernel's x86-specific halt and interrupt-control instructions are actual inline assembly. A future self-owned non-UEFI entry path, privilege-transition routines, exception/interrupt stubs, and task context switches **will require dedicated x86_64 assembly**; the repository has not implemented those yet.

Keep the Git repository at `D:\Desktop\Text_System` and continue developing in its UUID-named isolated worktree beneath `D:\Desktop\Text_System.worktrees\<worktree-id>`. All builds and ISO files go under the sibling `D:\Desktop\Text_System-Cache\`, keeping generated artifacts off C:. **The 30-day age limit conservatively preserves current builds.** Cleanup skips newer or locked files. It only examines regular files under the checked project build-cache and the signed-in user's `AppData\Local\Temp`; it never removes source, release ISO/EFI images, downloads, .NET/Rust/C++/Node toolchains, directories, Git data, or reparse points. Preview cleanup first:

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\cleanup-cache.ps1 -WhatIf
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\cleanup-cache.ps1
```

Optionally register an automatic, non-administrator, current-user Windows scheduled task for weekly Sunday cleanup at 03:00:

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\schedule-cache-cleanup.ps1
```

The task runs only when the account is signed in and starts missed runs on the next login. `-RemoveSchedule` removes this specific task. Both scripts verify the base repository, exact isolated Git worktree, and C: Temp location; never traverse junctions; leave directories, user data, and source untouched; and skip locked files.

The UEFI kernel intentionally uses Rust and assembly only. C#, JavaScript, and TypeScript are unnecessary dependencies in a freestanding EFI image; the project does not add Node packages or move existing system toolchains.

**This project does not reuse or import Redox, Serenity, Linux, or Limine kernel or bootloader code.** It has no Rust package dependencies. Its entry point and memory allocator are original implementations written here against the published x86_64 UEFI firmware interfaces.

## Build a bootable ISO

On Windows, install the stable GNU-hosted Rust toolchain, the `x86_64-unknown-uefi` target, and Python 3.11 or later. Then run:

```powershell
rustup default stable-x86_64-pc-windows-gnu
rustup target add x86_64-unknown-uefi
.\scripts\build.ps1
```

The build writes `D:\Desktop\Text_System-Cache\target\text-system.iso`. The ISO builder uses the Python standard library: it creates both an ISO 9660 volume and an El Torito UEFI boot entry backed by a FAT12 EFI System Partition with `EFI\BOOT\BOOTX64.EFI`. No separately downloaded bootloader, kernel image, Python package, or ISO-mastering executable is used.

Run the platform-independent memory-manager tests and ISO layout checks with:

```powershell
cargo test --locked
python -m unittest discover -s tests -v
```

## Boot in a virtual machine

Install the official QEMU x86_64 distribution with its EDK II OVMF UEFI firmware, build the ISO, then run:

```powershell
.\scripts\boot-qemu.ps1
```

QEMU is an optional validation environment; the kernel and ISO generator do not depend on it.

## Current scope and compatibility

This is an explicitly early bare-metal prototype, **not** an Ubuntu-compatible desktop operating system. It currently boots on x86_64 UEFI machines with an RGB or BGR Graphics Output Protocol framebuffer, uses the firmware memory map to initialize a coalescing buddy **physical-page allocator**, draws one static translucent desktop screen, and halts. The allocator supports up to 32 buddy orders, independently tracks managed, free, and allocated pages, rejects overlapping map regions and invalid/double frees, and coalesces adjacent free buddies.

There is not yet a scheduler, a virtual-memory manager or page-table switch, interrupt or exception handling, keyboard or pointer input, runnable processes, drivers, audio, networking, storage, filesystems, a window server or lock screen, adjustable appearance settings, an application-download system, AMD-specific graphics acceleration, Secure Boot signing, or Linux/Ubuntu binary compatibility. The screen is currently a boot illustration, **not** a working window manager or a fully implemented iPhone-style glass interface. UEFI GOP can display a framebuffer supplied by the motherboard firmware, but this prototype does not contain or load an AMD GPU driver. Future ISO images, UEFI boot support, physical memory management, and actual hardware compatibility must be validated separately before treating this as a stable or installable general-purpose system.
I don't know how to push it to Github, so I use Copilot.  
So you may see theContributors is Copilot.
