param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$videoNotesRoot = Split-Path -Parent $PSScriptRoot
$videoNotesToolsDir = Join-Path $videoNotesRoot "src-tauri\resources\tools"
$videoNotesLicensesDir = Join-Path $videoNotesToolsDir "licenses"
$videoNotesTempRoot = [System.IO.Path]::GetTempPath()
$videoNotesTempDir = Join-Path $videoNotesTempRoot ("video-notes-tools-" + [guid]::NewGuid().ToString("N"))

$ytDlpUrl = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
$ytDlpSumsUrl = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS"
$ffmpegArchiveUrl = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.7z"
$ffmpegSumUrl = "$ffmpegArchiveUrl.sha256"

function Receive-File {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    & curl.exe --location --fail --retry 3 --retry-delay 2 --output $Destination $Uri
    if ($LASTEXITCODE -ne 0) {
        throw "Download failed with exit code $LASTEXITCODE`: $Uri"
    }
}

function Get-ExpectedHash {
    param(
        [Parameter(Mandatory = $true)][string]$ChecksumFile,
        [string]$Filename
    )

    $lines = Get-Content -LiteralPath $ChecksumFile
    $selected = if ($Filename) {
        $lines | Where-Object { $_ -match ("\s[\*]?" + [regex]::Escape($Filename) + "$") } | Select-Object -First 1
    } else {
        $lines | Select-Object -First 1
    }
    if (-not $selected) {
        throw "Checksum entry not found for $Filename"
    }
    return (($selected.Trim() -split "\s+")[0]).ToLowerInvariant()
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

New-Item -ItemType Directory -Force -Path $videoNotesToolsDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesLicensesDir | Out-Null
New-Item -ItemType Directory -Force -Path $videoNotesTempDir | Out-Null

try {
    $ytDlpSums = Join-Path $videoNotesTempDir "SHA2-256SUMS"
    $ytDlpDownload = Join-Path $videoNotesTempDir "yt-dlp.exe"
    Receive-File -Uri $ytDlpSumsUrl -Destination $ytDlpSums
    $ytDlpExpected = Get-ExpectedHash -ChecksumFile $ytDlpSums -Filename "yt-dlp.exe"
    $ytDlpTarget = Join-Path $videoNotesToolsDir "yt-dlp.exe"
    $downloadYtDlp = $Force -or -not (Test-Path -LiteralPath $ytDlpTarget)
    if (-not $downloadYtDlp) {
        try { Assert-FileHash -Path $ytDlpTarget -Expected $ytDlpExpected } catch { $downloadYtDlp = $true }
    }
    if ($downloadYtDlp) {
        Receive-File -Uri $ytDlpUrl -Destination $ytDlpDownload
        Assert-FileHash -Path $ytDlpDownload -Expected $ytDlpExpected
        Copy-Item -LiteralPath $ytDlpDownload -Destination $ytDlpTarget -Force
    }
    Receive-File -Uri "https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/LICENSE" -Destination (Join-Path $videoNotesLicensesDir "yt-dlp-LICENSE.txt")
    Receive-File -Uri "https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/COPYING.GPLv3" -Destination (Join-Path $videoNotesLicensesDir "GPL-3.0.txt")

    $ffmpegArchive = Join-Path $videoNotesTempDir "ffmpeg-release-essentials.7z"
    $ffmpegSum = Join-Path $videoNotesTempDir "ffmpeg-release-essentials.7z.sha256"
    Receive-File -Uri $ffmpegSumUrl -Destination $ffmpegSum
    Receive-File -Uri $ffmpegArchiveUrl -Destination $ffmpegArchive
    $ffmpegExpected = Get-ExpectedHash -ChecksumFile $ffmpegSum
    Assert-FileHash -Path $ffmpegArchive -Expected $ffmpegExpected

    $ffmpegExtracted = Join-Path $videoNotesTempDir "ffmpeg"
    New-Item -ItemType Directory -Force -Path $ffmpegExtracted | Out-Null
    & tar.exe -xf $ffmpegArchive -C $ffmpegExtracted
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to extract the verified FFmpeg archive"
    }
    $ffmpegBinary = Get-ChildItem -LiteralPath $ffmpegExtracted -Filter "ffmpeg.exe" -File -Recurse | Select-Object -First 1
    $ffprobeBinary = Get-ChildItem -LiteralPath $ffmpegExtracted -Filter "ffprobe.exe" -File -Recurse | Select-Object -First 1
    if (-not $ffmpegBinary -or -not $ffprobeBinary) {
        throw "FFmpeg archive does not contain ffmpeg.exe and ffprobe.exe"
    }
    Copy-Item -LiteralPath $ffmpegBinary.FullName -Destination (Join-Path $videoNotesToolsDir "ffmpeg.exe") -Force
    Copy-Item -LiteralPath $ffprobeBinary.FullName -Destination (Join-Path $videoNotesToolsDir "ffprobe.exe") -Force
    $ffmpegReadme = Get-ChildItem -LiteralPath $ffmpegExtracted -Filter "README.txt" -File -Recurse | Select-Object -First 1
    if ($ffmpegReadme) {
        Copy-Item -LiteralPath $ffmpegReadme.FullName -Destination (Join-Path $videoNotesLicensesDir "FFmpeg-build-README.txt") -Force
    }

    $lock = [ordered]@{
        generatedAt = [DateTimeOffset]::UtcNow.ToString("O")
        ytDlp = [ordered]@{ source = $ytDlpUrl; sha256 = $ytDlpExpected }
        ffmpegArchive = [ordered]@{ source = $ffmpegArchiveUrl; sha256 = $ffmpegExpected }
    }
    $lock | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $videoNotesToolsDir "media-tools.lock.json") -Encoding utf8
    Write-Host "Media tools are ready in $videoNotesToolsDir"
}
finally {
    $resolvedTempRoot = [System.IO.Path]::GetFullPath($videoNotesTempRoot)
    $resolvedTempDir = [System.IO.Path]::GetFullPath($videoNotesTempDir)
    if ($resolvedTempDir.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedTempDir)) {
        Remove-Item -LiteralPath $resolvedTempDir -Recurse -Force
    }
}
