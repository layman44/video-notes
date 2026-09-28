<#
.SYNOPSIS
  把 landing/index.html 推送到腾讯云 CVM 并重载 Nginx。

.DESCRIPTION
  通过 scp 上传页面，再通过 ssh 落盘到站点目录并重载 Nginx。
  依赖本机 Windows 自带的 OpenSSH 客户端（scp / ssh）。
  首次使用建议先配置 SSH 免密，否则会提示输入服务器密码。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/deploy-landing.ps1 -Server 123.45.67.89 -Domain miniqmt.top
#>
param(
  [Parameter(Mandatory = $true)][string]$Server,          # 服务器公网 IP 或域名
  [string]$User = "root",
  [string]$Domain = "",                                    # 仅用于最后打印访问地址
  [string]$RemoteDir = "/var/www/vibecoding-studio",
  [string]$SourceDir = (Join-Path $PSScriptRoot "..\landing")
)

$ErrorActionPreference = "Stop"
$target = "$User@$Server"

$indexHtml = Join-Path $SourceDir "index.html"
if (-not (Test-Path $indexHtml)) { throw "找不到本地页面文件: $indexHtml" }
Write-Host "==> 上传 $SourceDir\* -> ${target}:/tmp/landing_dist"
ssh $target "rm -rf /tmp/landing_dist && mkdir -p /tmp/landing_dist"
scp -r "$SourceDir\*" "${target}:/tmp/landing_dist"
if ($LASTEXITCODE -ne 0) { throw "scp 上传失败" }

Write-Host "==> 服务器落盘并重载 Nginx"
$remoteCmd = "set -e; sudo mkdir -p $RemoteDir; sudo cp -rf /tmp/landing_dist/* $RemoteDir/; rm -rf /tmp/landing_dist; " +
             "(sudo chown -R www-data:www-data $RemoteDir 2>/dev/null || sudo chown -R nginx:nginx $RemoteDir); " +
             "sudo nginx -t && sudo systemctl reload nginx && echo '部署完成: $RemoteDir/'"
ssh $target $remoteCmd
if ($LASTEXITCODE -ne 0) { throw "远程部署失败" }

if ($Domain) { Write-Host "`n访问: http://$Domain" }
