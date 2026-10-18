param(
    [switch] $RemoveSchedule
)

$ErrorActionPreference = 'Stop'
$taskName = 'TextSystem-BuildCache-Cleanup'
. (Join-Path $PSScriptRoot 'project-paths.ps1')
$project = Get-TextSystemWorkspace -ScriptsDirectory $PSScriptRoot
$scriptPath = [System.IO.Path]::GetFullPath(
    (Join-Path $project.ProjectRepository 'scripts\cleanup-cache.ps1')
)
if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "Cache-cleanup script not found: $scriptPath"
}

if ($RemoveSchedule) {
    $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    if ($null -eq $task) {
        Write-Output "Scheduled task $taskName is not registered."
        exit 0
    }

    $action = $task.Actions | Select-Object -First 1
    $targetAction = ('-File "{0}"' -f $scriptPath)
    if ($action.Execute -notmatch 'powershell(\.exe)?$' -or
        $action.Arguments -notlike "*$targetAction*") {
        throw "Refusing to remove an unrelated task named $taskName."
    }

    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Output "Removed scheduled task $taskName."
    exit 0
}

$existingTask = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
if ($null -ne $existingTask) {
    throw "Scheduled task $taskName already exists; inspect it before changing or replacing it."
}

$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    throw "Windows PowerShell executable not found: $windowsPowerShell"
}

$userTempDirectory = [System.IO.Path]::GetFullPath(
    (Join-Path $env:USERPROFILE 'AppData\Local\Temp')
).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
if (-not (Test-Path -LiteralPath $userTempDirectory -PathType Container)) {
    throw "The user's standard temporary directory does not exist: $userTempDirectory"
}

$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction `
    -Execute $windowsPowerShell `
    -Argument ('-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $scriptPath)
$trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek Sunday -At '3:00AM'
$principal = New-ScheduledTaskPrincipal `
    -UserId $identity `
    -LogonType Interactive `
    -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
    -StartWhenAvailable `
    -MultipleInstances IgnoreNew `
    -ExecutionTimeLimit (New-TimeSpan -Minutes 30)

Register-ScheduledTask `
    -TaskName $taskName `
    -Action $action `
    -Trigger $trigger `
    -Principal $principal `
    -Settings $settings `
    -Description 'Remove only unlocked cache files older than 30 days inside the verified user Temp folder or D:\Desktop\Text_System-Cache.'

Write-Output "Registered current-user, non-elevated weekly cache cleanup. It runs Sundays at 03:00 when this Windows account is signed in."
