<#
.SYNOPSIS
构建并发布 Android APK 到 GitHub Release（自更新下载源）。

.DESCRIPTION
流程：读 Cargo.toml 版本 → 盖章 AndroidManifest（versionName / versionCode）
→ build.ps1 构建 arm64-v8a → 固定资产名上传 Release。

签名提醒：必须一直使用 android/.signing/local-test.keystore 签名。
签名变化后手机上无法覆盖安装，卸载重装还会丢 identity.json / session.json。

.EXAMPLE
./android/release.ps1 -AndroidRev 4                        # 发到 fork
./android/release.ps1 -AndroidRev 4 -Repo YanamiNeko/NekoSportsWorldTool -Notes "修复"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateRange(1, 99)][int]$AndroidRev,
    [string]$Repo = 'hb-degithub/NekoSportsWorldTool',
    [string]$Notes = '',
    [switch]$Draft,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $PSScriptRoot 'AndroidManifest.xml'

# ── 版本号（Cargo.toml 是唯一真值）─────────────────────────────
$cargoToml = Get-Content (Join-Path $projectRoot 'Cargo.toml') -Raw
if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'Cargo.toml 中找不到 version' }
$version = $Matches[1]
$parts = $version -split '\.' | ForEach-Object { [int]$_ }
if ($parts.Count -lt 3) { throw "版本号格式异常：$version" }
$versionName = "$version-android.$AndroidRev"
# 沿用现有编码 0.2.5-android.3 = 20503（major*1e6 + minor*1e4 + patch*1e2 + rev）
$versionCode = $parts[0] * 1000000 + $parts[1] * 10000 + $parts[2] * 100 + $AndroidRev
Write-Output "版本：$versionName（versionCode $versionCode）→ $Repo"

# ── 盖章 AndroidManifest ─────────────────────────────────────
$manifestText = Get-Content $manifest -Raw
$updated = $manifestText -replace 'android:versionCode="\d+"', ('android:versionCode="{0}"' -f $versionCode)
$updated = $updated -replace 'android:versionName="[^"]*"', ('android:versionName="{0}"' -f $versionName)
if ($updated -eq $manifestText -and $manifestText -notmatch [regex]::Escape(('android:versionName="{0}"' -f $versionName))) {
    throw 'AndroidManifest.xml 盖章失败（未匹配到 versionCode/versionName）'
}
[IO.File]::WriteAllText($manifest, $updated)
Write-Output "已盖章 AndroidManifest.xml"

# ── 构建 ────────────────────────────────────────────────────
if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build.ps1') -Abi arm64-v8a -Profile release
    if ($LASTEXITCODE -ne 0) { throw 'build.ps1 失败' }
}
$builtApk = Join-Path $PSScriptRoot 'build\arm64-v8a-release\NekoSportsWorldTool-arm64-v8a.apk'
if (-not (Test-Path -LiteralPath $builtApk)) { throw "找不到构建产物：$builtApk" }

# 固定资产名（更新器按此匹配；也兼容任意 *.apk 资产）
$asset = Join-Path $PSScriptRoot 'build\NekoSportsWorldTool-android-arm64.apk'
Copy-Item -LiteralPath $builtApk -Destination $asset -Force

# ── 发布 ────────────────────────────────────────────────────
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "未安装 gh CLI。手动发布：`ngh release create v$versionName --repo $Repo --latest `"$asset`""
}
if ([string]::IsNullOrWhiteSpace($Notes)) {
    $Notes = "NekoSportsWorldTool $versionName`n`n签名密钥未变，可直接覆盖安装。"
}
$tag = "v$versionName"
$arguments = @('release', 'create', $tag, '--repo', $Repo, '--latest',
    '--title', "NekoSportsWorldTool $versionName", '--notes', $Notes)
if ($Draft) { $arguments += '--draft' }
$arguments += $asset
Write-Output "gh $($arguments -join ' ')"
& gh @arguments
if ($LASTEXITCODE -ne 0) { throw 'gh release create 失败' }
Write-Output "√ 已发布 $tag → https://github.com/$Repo/releases/tag/$tag"
Write-Output '提醒：Android 端（关于页）会自动检查该 Release 并提示安装。'
