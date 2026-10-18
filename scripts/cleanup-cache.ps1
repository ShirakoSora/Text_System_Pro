[CmdletBinding(SupportsShouldProcess)]
param(
    [ValidateRange(1, 3650)]
    [int] $AgeDays = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'project-paths.ps1')
$project = Get-TextSystemWorkspace -ScriptsDirectory $PSScriptRoot -AllowRepository

$userTempDirectory = [System.IO.Path]::GetFullPath(
    (Join-Path $env:USERPROFILE 'AppData\Local\Temp')
).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
$configuredTempDirectory = [System.IO.Path]::GetFullPath(
    [System.IO.Path]::GetFullPath($env:TEMP)
).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
if (-not [string]::Equals(
        $configuredTempDirectory,
        $userTempDirectory,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
    throw "Refusing to clean unexpected user TEMP path: $configuredTempDirectory"
}

$cacheDirectory = [System.IO.DirectoryInfo]::new($project.CacheDirectory)
if ($cacheDirectory.Exists -and
    ($cacheDirectory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Refusing to clean build cache: D:\Desktop\Text_System-Cache is a reparse point.'
}
$approvedDirectories = @($userTempDirectory)
if ($cacheDirectory.Exists) {
    $approvedDirectories += $project.CacheDirectory
}
foreach ($approvedDirectory in $approvedDirectories) {
    $directory = [System.IO.DirectoryInfo]::new($approvedDirectory)
    if (-not $directory.Exists) {
        throw "Refusing to clean cache: approved cache directory does not exist: $approvedDirectory"
    }
    if (($directory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to clean cache: the approved directory is a reparse point: $approvedDirectory"
    }
}

$cutoff = [DateTime]::UtcNow.AddDays(-$AgeDays)
$removedBytes = [long] 0
$removedFiles = 0
$lockedFiles = 0
$failedFiles = 0
foreach ($approvedDirectory in $approvedDirectories) {
    $directoryRoot = [System.IO.Path]::GetFullPath($approvedDirectory).
        TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $directories = [System.Collections.Generic.Stack[string]]::new()
    $directories.Push($directoryRoot)

    while ($directories.Count -gt 0) {
        $currentDirectory = $directories.Pop()
        $children = $null
        try {
            $children = [System.IO.Directory]::EnumerateFileSystemEntries(
                $currentDirectory
            ).GetEnumerator()
            while ($children.MoveNext()) {
                $entryPath = $children.Current
                try {
                    $attributes = [System.IO.File]::GetAttributes($entryPath)
                    if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                        continue
                    }

                    if (($attributes -band [System.IO.FileAttributes]::Directory) -ne 0) {
                        $directories.Push($entryPath)
                        continue
                    }

                    $fullPath = [System.IO.Path]::GetFullPath($entryPath)
                    if (-not $fullPath.StartsWith(
                            "$directoryRoot\",
                            [System.StringComparison]::OrdinalIgnoreCase
                        )) {
                        throw "Refusing to process a file outside its approved cache: $fullPath"
                    }

                    $fileInfo = [System.IO.FileInfo]::new($fullPath)
                    $fileInfo.Refresh()
                    if ($fileInfo.LastWriteTimeUtc -ge $cutoff) {
                        continue
                    }

                    $fileBytes = $fileInfo.Length
                    $stream = $null
                    try {
                        $stream = [System.IO.File]::Open(
                            $fullPath,
                            [System.IO.FileMode]::Open,
                            [System.IO.FileAccess]::ReadWrite,
                            [System.IO.FileShare]::None
                        )
                    } catch [System.IO.IOException] {
                        $lockedFiles++
                        continue
                    } catch [System.UnauthorizedAccessException] {
                        $failedFiles++
                        continue
                    } finally {
                        if ($null -ne $stream) {
                            $stream.Dispose()
                        }
                    }

                    try {
                        if ($PSCmdlet.ShouldProcess($fullPath, 'Delete stale cache file')) {
                            [System.IO.File]::Delete($fullPath)
                            $removedFiles++
                            $removedBytes += $fileBytes
                        }
                    } catch [System.IO.IOException] {
                        $lockedFiles++
                    } catch [System.UnauthorizedAccessException] {
                        $failedFiles++
                    }
                } catch {
                    $failedFiles++
                    Write-Warning "Skipping cache entry: $entryPath ($($_.Exception.Message))"
                }
            }
        } catch {
            $failedFiles++
            Write-Warning "Skipping cache directory: $currentDirectory ($($_.Exception.Message))"
        } finally {
            if ($null -ne $children) {
                $children.Dispose()
            }
        }
    }
}

Write-Output "Removed $removedFiles files ($removedBytes bytes) older than $AgeDays days from the approved user Temp directory and D:\Desktop\Text_System-Cache."
Write-Output "Skipped $lockedFiles locked files and $failedFiles inaccessible or invalid entries."
