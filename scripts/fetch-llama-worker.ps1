param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$videoNotesRoot = Split-Path -Parent $PSScriptRoot
$videoNotesToolsDir = Join-Path $videoNotesRoot "src-tauri\resources\tools"
$videoNotesLlamaDir = Join-Path $videoNotesToolsDir "llama"
$videoNotesLicensesDir = Join-Path $videoNotesToolsDir "licenses"
$videoNotesTempRoot = [System.IO.Path]::GetTempPath()
$videoNotesTempDir = Join-Path $videoNotesTempRoot ("video-notes-llama-" + [guid]::NewGuid().ToString("N"))

$llamaVersion = "b10448"
$llamaArchiveName = "llama-b10448-bin-win-cpu-x64.zip"
$llamaArchiveUrl = "https://github.com/ggml-org/llama.cpp/releases/download/b10448/llama-b10448-bin-win-cpu-x64.zip"
$llamaArchiveSha256 = "9038c34d23769ac04a1f59835f41129f3810b3144bb8edc35183507baf827435"
$llamaLicenseUrl = "https://raw.githubusercontent.com/ggml-org/llama.cpp/$llamaVersion/LICENSE"

function Receive-File {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    & curl.exe --location --fail --retry 5 --retry-all-errors --retry-delay 2 --user-agent "VideoNotes build" --output $Destination $Uri
    if ($LASTEXITCODE -ne 0) {
        throw "Download failed with exit code $LASTEXITCODE`: $Uri"
    }
}

function Assert-FileHash {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path. Expected $Expected, received $actual"
    }
}

New-Item -ItemType Directory -Force -Path $videoNotesLlamaDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesLicensesDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesTempDir | Out-Null

try {
    $workerTarget = Join-Path $videoNotesLlamaDir "llama-cli.exe"
    if ($Force -or -not (Test-Path -LiteralPath $workerTarget)) {
        $archive = Join-Path $videoNotesTempDir $llamaArchiveName
        $extracted = Join-Path $videoNotesTempDir "extracted"
        Receive-File -Uri $llamaArchiveUrl -Destination $archive
        Assert-FileHash -Path $archive -Expected $llamaArchiveSha256
        Expand-Archive -LiteralPath $archive -DestinationPath $extracted -Force

        $worker = Get-ChildItem -LiteralPath $extracted -Filter "llama-cli.exe" -File -Recurse | Select-Object -First 1
        if (-not $worker) {
            throw "Official llama.cpp archive does not contain llama-cli.exe"
        }
        Copy-Item -LiteralPath $worker.FullName -Destination $workerTarget -Force

        Get-ChildItem -LiteralPath $worker.DirectoryName -Filter "*.dll" -File | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $videoNotesLlamaDir $_.Name) -Force
        }
    }

    Receive-File -Uri $llamaLicenseUrl -Destination (Join-Path $videoNotesLicensesDir "llama.cpp-LICENSE.txt")
    $lock = [ordered]@{
        version = $llamaVersion
        source = $llamaArchiveUrl
        sha256 = $llamaArchiveSha256
    }
    $lock | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $videoNotesLlamaDir "llama-worker.lock.json") -Encoding utf8

    & $workerTarget --version
    if ($LASTEXITCODE -ne 0) {
        throw "llama-cli.exe failed its startup check"
    }
    Write-Host "llama.cpp CPU worker is ready in $videoNotesLlamaDir"
}
finally {
    $resolvedTempRoot = [System.IO.Path]::GetFullPath($videoNotesTempRoot)
    $resolvedTempDir = [System.IO.Path]::GetFullPath($videoNotesTempDir)
    if ($resolvedTempDir.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedTempDir)) {
        Remove-Item -LiteralPath $resolvedTempDir -Recurse -Force
    }
}
