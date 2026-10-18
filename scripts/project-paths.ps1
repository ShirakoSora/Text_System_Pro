function Get-TextSystemWorkspace {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $ScriptsDirectory,
        [switch] $AllowRepository
    )

    $projectRepository = [System.IO.Path]::GetFullPath('D:\Desktop\Text_System').
        TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $worktreesRoot = [System.IO.Path]::GetFullPath('D:\Desktop\Text_System.worktrees').
        TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $cacheDirectory = [System.IO.Path]::GetFullPath('D:\Desktop\Text_System-Cache').
        TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $workspaceDirectory = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::GetDirectoryName(
            [System.IO.Path]::GetFullPath($ScriptsDirectory)
        )
    ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)

    $requestedScriptsDirectory = [System.IO.Path]::GetFullPath(
        $ScriptsDirectory
    ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $repositoryScriptsDirectory = [System.IO.Path]::GetFullPath(
        (Join-Path $projectRepository 'scripts')
    ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $isStableRepositoryScripts = [string]::Equals(
        $requestedScriptsDirectory,
        $repositoryScriptsDirectory,
        [System.StringComparison]::OrdinalIgnoreCase
    )
    if ($isStableRepositoryScripts -and -not $AllowRepository) {
        throw 'Run development scripts from the isolated Git worktree, not the shared repository.'
    }
    if (-not $isStableRepositoryScripts -and
        [string]::Equals(
            $workspaceDirectory,
            $projectRepository,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'Only the verified cleanup scripts may run directly from the stable repository.'
    }

    $requiredDirectories = @($projectRepository)
    if ($isStableRepositoryScripts) {
        $requiredDirectories += $workspaceDirectory
    } else {
        $requiredDirectories += @($worktreesRoot, $workspaceDirectory)
    }
    foreach ($requiredDirectory in $requiredDirectories) {
        $directoryInfo = [System.IO.DirectoryInfo]::new($requiredDirectory)
        if (-not $directoryInfo.Exists) {
            throw "Expected Text System Git directory is missing: $requiredDirectory"
        }
        if (($directoryInfo.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to trust a reparse-point Text System directory: $requiredDirectory"
        }
    }

    $repositoryParent = [System.IO.Path]::GetDirectoryName($projectRepository).
        TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    if (-not [string]::Equals(
            $repositoryParent,
            [System.IO.Path]::GetDirectoryName($worktreesRoot).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar
            ),
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'Text System repository and worktrees must both be direct children of D:\Desktop.'
    }

    $gitDirectory = Join-Path $projectRepository '.git'
    if (-not [System.IO.Directory]::Exists($gitDirectory) -or
        (([System.IO.File]::GetAttributes($gitDirectory) -band
            [System.IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw "The original Git repository is not a normal, local .git directory: $gitDirectory"
    }
    $scriptsDirectoryInfo = [System.IO.DirectoryInfo]::new($ScriptsDirectory)
    if (-not $scriptsDirectoryInfo.Exists -or
        ($scriptsDirectoryInfo.Attributes -band
            [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The project scripts directory is missing or unsafe.'
    }
    if ($isStableRepositoryScripts) {
        return [pscustomobject]@{
            ProjectRepository = $projectRepository
            WorktreesRoot     = $worktreesRoot
            Workspace         = $projectRepository
            WorktreeId        = $null
            CacheDirectory    = $cacheDirectory
        }
    }

    $gitWorktreesDirectory = Join-Path $gitDirectory 'worktrees'
    if (-not [System.IO.Directory]::Exists($gitWorktreesDirectory) -or
        (([System.IO.File]::GetAttributes($gitWorktreesDirectory) -band
            [System.IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw "The Git worktree metadata directory is unavailable or unsafe: $gitWorktreesDirectory"
    }

    if (-not [string]::Equals(
            [System.IO.Path]::GetDirectoryName($workspaceDirectory).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar
            ),
            $worktreesRoot,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw "Refusing to run from outside the approved isolated worktrees: $workspaceDirectory"
    }

    $worktreeId = [System.IO.Path]::GetFileName($workspaceDirectory)
    if ($worktreeId -notmatch '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
        throw 'Refusing to trust an isolated-worktree directory without a canonical GUID name.'
    }

    $metadataPath = Join-Path $workspaceDirectory '.git'
    if (-not [System.IO.File]::Exists($metadataPath) -or
        [System.IO.Directory]::Exists($metadataPath)) {
        throw "Expected the isolated Git worktree metadata file: $metadataPath"
    }

    $metadata = [System.IO.File]::ReadAllText($metadataPath).Trim()
    if ($metadata -notmatch '^gitdir:\s*(.+)$') {
        throw "Invalid isolated worktree Git metadata file: $metadataPath"
    }

    $actualGitDirectory = $matches[1].Replace('/', '\')
    if (-not [System.IO.Path]::IsPathRooted($actualGitDirectory)) {
        $actualGitDirectory = [System.IO.Path]::Combine($workspaceDirectory, $actualGitDirectory)
    }
    $actualGitDirectory = [System.IO.Path]::GetFullPath(
        $actualGitDirectory
    ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $expectedGitDirectory = [System.IO.Path]::GetFullPath(
        (Join-Path $projectRepository ".git\worktrees\$worktreeId")
    ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)

    if (-not [string]::Equals(
            $actualGitDirectory,
            $expectedGitDirectory,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'Isolated worktree does not belong to the approved Text System D: drive repository.'
    }
    $adminDirectory = [System.IO.DirectoryInfo]::new($expectedGitDirectory)
    if (-not $adminDirectory.Exists -or
        ($adminDirectory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The verified Git worktree administration directory is missing or unsafe.'
    }

    $expectedScriptsDirectory = Join-Path $workspaceDirectory 'scripts'
    if (-not [string]::Equals(
            [System.IO.Path]::GetFullPath($ScriptsDirectory).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar
            ),
            [System.IO.Path]::GetFullPath($expectedScriptsDirectory).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar
            ),
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'The build or cleanup script must run from the approved worktree scripts directory.'
    }

    [pscustomobject]@{
        ProjectRepository = $projectRepository
        WorktreesRoot     = $worktreesRoot
        Workspace         = $workspaceDirectory
        WorktreeId        = $worktreeId
        CacheDirectory    = $cacheDirectory
    }
}
