param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$videoNotesRoot = Split-Path -Parent $PSScriptRoot
$videoNotesToolsDir = Join-Path $videoNotesRoot "src-tauri\resources\tools"
$videoNotesWhisperDir = Join-Path $videoNotesToolsDir "whisper"
$videoNotesLicensesDir = Join-Path $videoNotesToolsDir "licenses"
$videoNotesTempRoot = [System.IO.Path]::GetTempPath()
$videoNotesTempDir = Join-Path $videoNotesTempRoot ("video-notes-whisper-" + [guid]::NewGuid().ToString("N"))

$whisperVersion = "v1.9.2"
$whisperArchiveName = "whisper-bin-x64.zip"
$whisperArchiveUrl = "https://api.github.com/repos/ggml-org/whisper.cpp/releases/assets/501504923"
$whisperArchiveSha256 = "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a"
$whisperLicenseUrl = "https://raw.githubusercontent.com/ggml-org/whisper.cpp/$whisperVersion/LICENSE"

function Receive-File {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    & curl.exe --location --fail --retry 5 --retry-all-errors --retry-delay 2 --user-agent "VideoNotes build" --header "Accept: application/octet-stream" --output $Destination $Uri
    if ($LASTEXITCODE -ne 0) {
        throw "Download failed with exit code $LASTEXITCODE`: $Uri"
    }
}

function Assert-FileHash {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            $actual = ([System.BitConverter]::ToString($sha.ComputeHash($stream))).Replace("-", "").ToLowerInvariant()
        }
        finally {
            $sha.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path. Expected $Expected, received $actual"
    }
}

New-Item -ItemType Directory -Force -Path $videoNotesToolsDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesWhisperDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesLicensesDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesTempDir | Out-Null

try {
    $workerTarget = Join-Path $videoNotesWhisperDir "whisper-cli.exe"
    $runtimeComplete =
        (Test-Path -LiteralPath $workerTarget) -and
        (Test-Path -LiteralPath (Join-Path $videoNotesWhisperDir "whisper.dll")) -and
        (Test-Path -LiteralPath (Join-Path $videoNotesWhisperDir "ggml.dll")) -and
        (Test-Path -LiteralPath (Join-Path $videoNotesWhisperDir "ggml-base.dll")) -and
        [bool](Get-ChildItem -LiteralPath $videoNotesWhisperDir -Filter "ggml-cpu-*.dll" -File -ErrorAction SilentlyContinue | Select-Object -First 1)

    if ($Force -or -not $runtimeComplete) {
        $archive = Join-Path $videoNotesTempDir $whisperArchiveName
        $extracted = Join-Path $videoNotesTempDir "extracted"
        Receive-File -Uri $whisperArchiveUrl -Destination $archive
        Assert-FileHash -Path $archive -Expected $whisperArchiveSha256
        Expand-Archive -LiteralPath $archive -DestinationPath $extracted -Force

        $worker = Get-ChildItem -LiteralPath $extracted -Filter "whisper-cli.exe" -File -Recurse | Select-Object -First 1
        if (-not $worker) {
            throw "Official whisper.cpp archive does not contain whisper-cli.exe"
        }
        Copy-Item -LiteralPath $worker.FullName -Destination $workerTarget -Force

        $runtimeFiles = Get-ChildItem -LiteralPath $worker.Directory.FullName -File | Where-Object {
            $_.Name -eq "whisper.dll" -or $_.Name -like "ggml*.dll"
        }
        foreach ($runtimeFile in $runtimeFiles) {
            Copy-Item -LiteralPath $runtimeFile.FullName -Destination (Join-Path $videoNotesWhisperDir $runtimeFile.Name) -Force
        }
    }

    $missingRuntime = @("whisper.dll", "ggml.dll", "ggml-base.dll") | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $videoNotesWhisperDir $_))
    }
    $cpuBackends = Get-ChildItem -LiteralPath $videoNotesWhisperDir -Filter "ggml-cpu-*.dll" -File -ErrorAction SilentlyContinue
    if ($missingRuntime.Count -gt 0 -or $cpuBackends.Count -eq 0) {
        throw "whisper.cpp runtime is incomplete: missing core DLLs or GGML CPU backend variants"
    }

    Receive-File -Uri $whisperLicenseUrl -Destination (Join-Path $videoNotesLicensesDir "whisper.cpp-LICENSE.txt")
    $lock = [ordered]@{
        version = $whisperVersion
        source = $whisperArchiveUrl
        sha256 = $whisperArchiveSha256
    }
    $lock | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $videoNotesWhisperDir "whisper-worker.lock.json") -Encoding utf8

    $startupStdout = Join-Path $videoNotesTempDir "whisper-startup.stdout.txt"
    $startupStderr = Join-Path $videoNotesTempDir "whisper-startup.stderr.txt"
    $startupProcess = Start-Process -FilePath $workerTarget -ArgumentList @("--no-gpu", "--version") -NoNewWindow -Wait -PassThru -RedirectStandardOutput $startupStdout -RedirectStandardError $startupStderr
    $startupOutput = @(
        Get-Content -LiteralPath $startupStdout -ErrorAction SilentlyContinue
        Get-Content -LiteralPath $startupStderr -ErrorAction SilentlyContinue
    )
    if ($startupProcess.ExitCode -ne 0) {
        throw "whisper-cli.exe failed its startup check"
    }
    if (($startupOutput -join "`n") -notmatch "loaded CPU backend") {
        throw "whisper-cli.exe did not load a GGML CPU backend"
    }
    $startupOutput | Write-Host
    Write-Host "whisper.cpp CPU worker is ready in $videoNotesWhisperDir"
}
finally {
    $resolvedTempRoot = [System.IO.Path]::GetFullPath($videoNotesTempRoot)
    $resolvedTempDir = [System.IO.Path]::GetFullPath($videoNotesTempDir)
    if ($resolvedTempDir.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedTempDir)) {
        Remove-Item -LiteralPath $resolvedTempDir -Recurse -Force
    }
}
