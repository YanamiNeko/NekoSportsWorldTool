[CmdletBinding()]
param(
    [ValidateSet('arm64-v8a', 'x86_64')][string]$Abi = 'arm64-v8a',
    [ValidateSet('debug', 'release')][string]$Profile = 'release',
    [string]$SdkPath = $env:ANDROID_HOME,
    [string]$NdkVersion = '29.0.14206865',
    [string]$BuildToolsVersion = '36.0.0',
    [string]$Platform = 'android-36',
    [string]$TargetDir = '',
    [switch]$SkipRust
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $SdkPath) { $SdkPath = Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $TargetDir) {
    $projectDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($projectRoot))).Substring(0, 12)
    $TargetDir = Join-Path $env:TEMP "neko-android-$projectDigest"
}
if ($TargetDir -match '[^\x00-\x7F]') { throw 'NDK build cache path must contain ASCII characters only; pass -TargetDir with a suitable path.' }
$buildTools = Join-Path $SdkPath "build-tools\$BuildToolsVersion"
$ndkBin = Join-Path $SdkPath "ndk\$NdkVersion\toolchains\llvm\prebuilt\windows-x86_64\bin"
$androidJar = Join-Path $SdkPath "platforms\$Platform\android.jar"
$javaBin = Split-Path (Get-Command javac -ErrorAction Stop).Source
$targetTriple = if ($Abi -eq 'arm64-v8a') { 'aarch64-linux-android' } else { 'x86_64-linux-android' }
$targetKey = $targetTriple.Replace('-', '_').ToUpperInvariant()
$deliveryDir = Join-Path $PSScriptRoot "build\$Abi-$Profile"
$outputDir = Join-Path $TargetDir "package\$Abi-$Profile"
$classesDir = Join-Path $outputDir 'classes'
$dexDir = Join-Path $outputDir 'dex'
$stagingDir = Join-Path $outputDir 'apk'
$signingDir = Join-Path $PSScriptRoot '.signing'
foreach ($toolPath in @($androidJar, (Join-Path $buildTools 'aapt2.exe'), (Join-Path $ndkBin 'llvm-readelf.exe'))) {
    if (-not (Test-Path -LiteralPath $toolPath)) { throw "Missing Android tool: $toolPath" }
}
foreach ($directory in @($classesDir,$dexDir,$stagingDir,$signingDir,$deliveryDir)) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
# aapt2 and the NDK Windows wrappers do not reliably accept non-ASCII paths.
$packageSources = Join-Path $outputDir 'source'
New-Item -ItemType Directory -Force -Path $packageSources | Out-Null
foreach ($sourceName in @('java','res','AndroidManifest.xml')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $sourceName) -Destination $packageSources -Recurse -Force
}

function Invoke-Checked([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Executable exited with $LASTEXITCODE" }
}

# Scope all toolchain environment changes to this script's execution.
$savedEnvironment = @{}
$buildEnvironment = @{
    'CARGO_TARGET_DIR' = $TargetDir
    "CARGO_TARGET_${targetKey}_LINKER" = (Join-Path $ndkBin "${targetTriple}26-clang.cmd")
    "CC_$($targetTriple.Replace('-', '_'))" = (Join-Path $ndkBin "${targetTriple}26-clang.cmd")
    "AR_$($targetTriple.Replace('-', '_'))" = (Join-Path $ndkBin 'llvm-ar.exe')
    "CARGO_TARGET_${targetKey}_RUSTFLAGS" = '-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384'
}
try {
    foreach ($entry in $buildEnvironment.GetEnumerator()) {
        $savedEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
        [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
    }
    Push-Location $projectRoot
    try {
        if (-not $SkipRust) {
            $cargoArgs = @('build','--locked','--target',$targetTriple,'--features','android','--lib')
            if ($Profile -eq 'release') { $cargoArgs += '--release' }
            Invoke-Checked 'cargo' $cargoArgs
        }
    } finally { Pop-Location }
} finally {
    foreach ($entry in $savedEnvironment.GetEnumerator()) { [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process') }
}
$nativeLibrary = Join-Path $TargetDir "$targetTriple\$Profile\libnekosportsworldtool.so"
if (-not (Test-Path -LiteralPath $nativeLibrary)) { throw "Missing native library: $nativeLibrary" }

$programHeaders = & (Join-Path $ndkBin 'llvm-readelf.exe') '-lW' $nativeLibrary
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect ELF headers' }
$loadHeaders = @($programHeaders | Where-Object { $_ -match '^\s*LOAD\s' })
if ($loadHeaders.Count -eq 0) { throw 'No ELF LOAD headers found' }
foreach ($header in $loadHeaders) {
    $alignment = ($header.Trim() -split '\s+')[-1]
    if ([Convert]::ToInt64($alignment.Substring(2), 16) -lt 16384) { throw "ELF is not 16 KB aligned: $header" }
}

$sources = @(Get-ChildItem -LiteralPath (Join-Path $packageSources 'java') -Recurse -Filter '*.java' | Select-Object -ExpandProperty FullName)
$androidBootClasspath = $androidJar + [IO.Path]::PathSeparator + (Join-Path $buildTools 'core-lambda-stubs.jar')
Invoke-Checked (Join-Path $javaBin 'javac.exe') (@('-encoding','UTF-8','-source','8','-target','8','-Xlint:-options','-bootclasspath',$androidBootClasspath,'-d',$classesDir) + $sources)
$classJar = Join-Path $outputDir 'classes.jar'
Invoke-Checked (Join-Path $javaBin 'jar.exe') @('cf',$classJar,'-C',$classesDir,'.')
Invoke-Checked (Join-Path $buildTools 'd8.bat') @('--lib',$androidJar,'--min-api','26','--output',$dexDir,$classJar)

$resourcesZip = Join-Path $outputDir 'resources.zip'
Invoke-Checked (Join-Path $buildTools 'aapt2.exe') @('compile','--dir',(Join-Path $packageSources 'res'),'-o',$resourcesZip)
$unsignedApk = Join-Path $outputDir 'unsigned.apk'
Invoke-Checked (Join-Path $buildTools 'aapt2.exe') @('link','-o',$unsignedApk,'-I',$androidJar,'--manifest',(Join-Path $packageSources 'AndroidManifest.xml'),$resourcesZip)

# Add only the declared native library and dex. No workspace JSON or credentials.
$libDir = Join-Path $stagingDir "lib\$Abi"
New-Item -ItemType Directory -Force -Path $libDir | Out-Null
Copy-Item -LiteralPath $nativeLibrary -Destination (Join-Path $libDir 'libnekosportsworldtool.so') -Force
Copy-Item -LiteralPath (Join-Path $dexDir 'classes.dex') -Destination (Join-Path $stagingDir 'classes.dex') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::Open($unsignedApk, [IO.Compression.ZipArchiveMode]::Update)
try {
    foreach ($relative in @('classes.dex', "lib/$Abi/libnekosportsworldtool.so")) {
        $oldEntry = $zip.GetEntry($relative)
        if ($oldEntry) { $oldEntry.Delete() }
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $stagingDir $relative), $relative, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally { $zip.Dispose() }
$alignedApk = Join-Path $outputDir 'aligned.apk'
Invoke-Checked (Join-Path $buildTools 'zipalign.exe') @('-f','-P','16','4',$unsignedApk,$alignedApk)
$keyStore = Join-Path $signingDir 'local-test.keystore'
if (-not (Test-Path -LiteralPath $keyStore)) {
    Invoke-Checked (Join-Path $javaBin 'keytool.exe') @('-genkeypair','-keystore',$keyStore,'-storepass','android','-keypass','android','-alias','androiddebugkey','-keyalg','RSA','-keysize','2048','-validity','10000','-dname','CN=Android Debug,O=Android,C=US')
}
$finalApk = Join-Path $outputDir "NekoSportsWorldTool-$Abi.apk"
Invoke-Checked (Join-Path $buildTools 'apksigner.bat') @('sign','--ks',$keyStore,'--ks-key-alias','androiddebugkey','--ks-pass','pass:android','--key-pass','pass:android','--out',$finalApk,$alignedApk)
Invoke-Checked (Join-Path $buildTools 'apksigner.bat') @('verify','--verbose',$finalApk)
Invoke-Checked (Join-Path $buildTools 'zipalign.exe') @('-c','-P','16','4',$finalApk)
$manifestInfo = & (Join-Path $buildTools 'aapt2.exe') 'dump' 'badging' $finalApk
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect packaged manifest' }
$manifestInfo | Select-String 'package:|sdkVersion:|targetSdkVersion:|native-code:'
$deliveredApk = Join-Path $deliveryDir (Split-Path $finalApk -Leaf)
Copy-Item -LiteralPath $finalApk -Destination $deliveredApk -Force
Get-FileHash -LiteralPath $deliveredApk -Algorithm SHA256
Write-Output "APK: $deliveredApk"
