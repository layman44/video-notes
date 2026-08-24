param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$projectRoot = Split-Path -Parent $PSScriptRoot
$toolsDir = Join-Path $projectRoot "src-tauri\resources\tools\openasr"
$version = "0.1.30"
$archiveName = "openasr-$version-windows-x86_64.zip"
$archiveUrl = "https://github.com/QuintinShaw/openasr/releases/download/v$version/$archiveName"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("video-notes-openasr-" + [guid]::NewGuid().ToString("N"))
$bundleDir = $toolsDir
$target = Join-Path $bundleDir "openasr.exe"

New-Item -ItemType Directory -Force -Path $toolsDir, $tempDir | Out-Null
try {
    if ($Force -or -not (Test-Path -LiteralPath $target)) {
        $archive = Join-Path $tempDir $archiveName
        & curl.exe --location --fail --retry 5 --retry-all-errors --retry-delay 2 --user-agent "VideoNotes build" --output $archive $archiveUrl
        if ($LASTEXITCODE -ne 0) { throw "OpenASR 下载失败：$archiveUrl" }
        $archiveSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        [ordered]@{ version = $version; source = $archiveUrl; sha256 = $archiveSha256 } |
            ConvertTo-Json -Depth 3 |
            Set-Content -LiteralPath (Join-Path $toolsDir "openasr-worker.lock.json") -Encoding utf8
        $extracted = Join-Path $tempDir "extracted"
        Expand-Archive -LiteralPath $archive -DestinationPath $extracted -Force
        $worker = Get-ChildItem -LiteralPath $extracted -Filter "openasr.exe" -File -Recurse | Select-Object -First 1
        if (-not $worker) { throw "OpenASR 压缩包中没有找到 openasr.exe" }
        New-Item -ItemType Directory -Force -Path $bundleDir | Out-Null
        Copy-Item -LiteralPath $worker.FullName -Destination $target -Force
        Get-ChildItem -LiteralPath $worker.DirectoryName -File | Where-Object { $_.Extension -in @(".dll", ".pdb") } | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $bundleDir $_.Name) -Force
        }
    }
    & $target --version
    if ($LASTEXITCODE -ne 0) { throw "openasr.exe 启动检查失败" }
    Write-Host "OpenASR $version 已安装到 $target" -ForegroundColor Green
}
finally {
    if (Test-Path -LiteralPath $tempDir) { Remove-Item -LiteralPath $tempDir -Recurse -Force }
}
