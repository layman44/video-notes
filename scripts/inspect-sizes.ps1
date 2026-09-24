$list = @(
  @{ Name = "FFmpeg (ffmpeg.exe)"; Size = (Get-Item "src-tauri/resources/tools/ffmpeg.exe").Length },
  @{ Name = "ffprobe (ffprobe.exe)"; Size = (Get-Item "src-tauri/resources/tools/ffprobe.exe").Length },
  @{ Name = "OpenASR (openasr.exe + DLLs)"; Size = (Get-ChildItem "src-tauri/resources/tools/openasr" -Recurse -File | Measure-Object -Property Length -Sum).Sum },
  @{ Name = "llama.cpp (llama-cli.exe + DLLs)"; Size = (Get-ChildItem "src-tauri/resources/tools/llama" -Recurse -File | Measure-Object -Property Length -Sum).Sum },
  @{ Name = "yt-dlp (yt-dlp.exe)"; Size = (Get-Item "src-tauri/resources/tools/yt-dlp.exe").Length },
  @{ Name = "主程序 (video-notes.exe)"; Size = (Get-Item "src-tauri/target/release/video-notes.exe").Length }
)
$sum = ($list | Measure-Object -Property Size -Sum).Sum
foreach ($item in $list) {
  $mb = [math]::Round($item.Size / 1048576, 2)
  $pct = [math]::Round(($item.Size / $sum) * 100, 1)
  Write-Host ("- " + $item.Name + ": " + $mb + " MB (" + $pct + "%)")
}
Write-Host ("- 总未压缩体积: " + [math]::Round($sum / 1048576, 2) + " MB")
