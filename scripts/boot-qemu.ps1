$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'project-paths.ps1')
$project = Get-TextSystemWorkspace -ScriptsDirectory $PSScriptRoot
$isoPath = Join-Path $project.CacheDirectory 'target\text-system.iso'
if (-not (Test-Path -LiteralPath $isoPath -PathType Leaf)) {
    throw "Bootable ISO not found; run scripts\build.ps1 first: $isoPath"
}

$qemu = Get-Command 'qemu-system-x86_64' -ErrorAction SilentlyContinue
if ($null -eq $qemu) {
    throw 'QEMU is not installed. Install the official SoftwareFreedomConservancy.QEMU package, then open a new terminal.'
}

$firmwareCandidates = @(
    (Join-Path (Split-Path -Parent $qemu.Source) 'share\qemu\edk2-x86_64-code.fd'),
    (Join-Path (Split-Path -Parent $qemu.Source) 'share\edk2-x86_64-code.fd')
)
$firmware = $firmwareCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if ($null -eq $firmware) {
    throw 'UEFI firmware OVMF_CODE was not found next to QEMU; supply the firmware path explicitly.'
}

& $qemu.Source `
    -machine q35 `
    -m 512M `
    -bios $firmware `
    -cdrom $isoPath `
    -boot d
if ($LASTEXITCODE -ne 0) {
    throw "QEMU stopped with exit code $LASTEXITCODE."
}
