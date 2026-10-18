$ErrorActionPreference = 'Stop'

$workspace = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'project-paths.ps1')
$paths = Get-TextSystemWorkspace -ScriptsDirectory $PSScriptRoot
$target = 'x86_64-unknown-uefi'
$buildRoot = Join-Path $paths.CacheDirectory 'target'
$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'

& rustc --version
if ($LASTEXITCODE -ne 0) {
    throw 'A working stable-x86_64-pc-windows-gnu Rust toolchain is required.'
}

$installedTargets = & rustup target list --installed
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to query installed Rust compilation targets.'
}
if ($installedTargets -notcontains $target) {
    throw "Rust target $target is not installed. Install it with: rustup target add $target"
}

$rustcSysroot = (& rustc --print sysroot).Trim()
$hostTriple = (& rustc -vV | Select-String '^host: ').ToString().Split(' ')[1]
$linkerDirectory = Join-Path $rustcSysroot "lib\rustlib\$hostTriple\bin"
$env:PATH = "$linkerDirectory;$env:PATH"

Push-Location $workspace
try {
    cargo test --locked --target-dir $buildRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'The host memory-manager tests failed.'
    }

    cargo build --release --locked --target $target --bin text-kernel --target-dir $buildRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Building the UEFI kernel failed.'
    }

    $efiApplication = Join-Path $buildRoot "$target\release\text-kernel.efi"
    if (-not (Test-Path -LiteralPath $efiApplication -PathType Leaf)) {
        throw "The Rust compiler did not produce a UEFI application at $efiApplication."
    }

    $isoPath = Join-Path $buildRoot 'text-system.iso'
    python (Join-Path $workspace 'scripts\make_iso.py') `
        --efi $efiApplication `
        --out $isoPath `
        --name TEXTOSYS
    if ($LASTEXITCODE -ne 0) {
        throw 'Creating the UEFI bootable ISO failed.'
    }
    Write-Output 'Inspecting EFI application PE/COFF header...'
    $peInspector = Join-Path $linkerDirectory 'llvm-readobj.exe'
    if (-not (Test-Path -LiteralPath $peInspector -PathType Leaf)) {
        throw "Cannot verify the EFI image; PE inspector not found: $peInspector"
    }
    $peHeaders = & $peInspector --file-headers $efiApplication
    if ($LASTEXITCODE -ne 0) {
        throw 'Verifying the EFI application PE/COFF header failed.'
    }
    if ($peHeaders -notmatch 'Subsystem: IMAGE_SUBSYSTEM_EFI_APPLICATION') {
        throw 'The compiled EFI image does not declare the EFI Application subsystem.'
    }
    $peHeaders | Select-String 'File:|Format:|Arch:|Machine:|AddressOfEntryPoint:|Subsystem:'
} finally {
    Pop-Location
}
