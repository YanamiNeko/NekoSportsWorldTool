[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^emulator-[0-9]+$')][string]$Serial,
    [string]$SdkPath = (Join-Path $env:LOCALAPPDATA 'Android\Sdk')
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($projectRoot))).Substring(0, 12)
$cacheRoot = Join-Path $env:TEMP "neko-android-$digest"
$output = Join-Path $cacheRoot 'instrumentation'
New-Item -ItemType Directory -Force -Path $output,(Join-Path $output 'classes'),(Join-Path $output 'dex') | Out-Null
$tools = Join-Path $SdkPath 'build-tools\36.0.0'
$androidJar = Join-Path $SdkPath 'platforms\android-36\android.jar'
$javaBin = Split-Path (Get-Command javac -ErrorAction Stop).Source
$appClasses = Join-Path $cacheRoot 'package\x86_64-debug\classes.jar'
$adb = Join-Path $SdkPath 'platform-tools\adb.exe'
function Invoke-Checked([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Executable exited with $LASTEXITCODE" }
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tests\SmokeInstrumentation.java') -Destination $output -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tests\AndroidManifest.xml') -Destination $output -Force
$bootClasspath = $androidJar + ';' + (Join-Path $tools 'core-lambda-stubs.jar')
Invoke-Checked (Join-Path $javaBin 'javac.exe') @('-encoding','UTF-8','-source','8','-target','8','-Xlint:-options','-bootclasspath',$bootClasspath,'-classpath',$appClasses,'-d',(Join-Path $output 'classes'),(Join-Path $output 'SmokeInstrumentation.java'))
Invoke-Checked (Join-Path $javaBin 'jar.exe') @('cf',(Join-Path $output 'tests.jar'),'-C',(Join-Path $output 'classes'),'.')
Invoke-Checked (Join-Path $tools 'd8.bat') @('--lib',$androidJar,'--classpath',$appClasses,'--min-api','26','--output',(Join-Path $output 'dex'),(Join-Path $output 'tests.jar'))
$unsigned = Join-Path $output 'unsigned.apk'
Invoke-Checked (Join-Path $tools 'aapt2.exe') @('link','-o',$unsigned,'-I',$androidJar,'--manifest',(Join-Path $output 'AndroidManifest.xml'))
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::Open($unsigned, [IO.Compression.ZipArchiveMode]::Update)
try { [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $output 'dex\classes.dex'), 'classes.dex') | Out-Null }
finally { $zip.Dispose() }
$testApk = Join-Path $output 'tests.apk'
Invoke-Checked (Join-Path $tools 'apksigner.bat') @('sign','--ks',(Join-Path $PSScriptRoot '.signing\local-test.keystore'),'--ks-pass','pass:android','--key-pass','pass:android','--out',$testApk,$unsigned)
Invoke-Checked $adb @('-s',$Serial,'install','-r',$testApk)
Invoke-Checked $adb @('-s',$Serial,'shell','input','keyevent','224')
Invoke-Checked $adb @('-s',$Serial,'shell','wm','dismiss-keyguard')
$result = & $adb '-s' $Serial 'shell' 'am' 'instrument' '-w' 'org.nekosportsworld.tool.tests/org.nekosportsworld.tool.tests.SmokeInstrumentation'
$result
if ($LASTEXITCODE -ne 0 -or ($result -join "`n") -notmatch 'PASS: [0-9]+ Android integration checks') { throw 'Android integration checks failed' }
$restartResult = & $adb '-s' $Serial 'shell' 'am' 'instrument' '-w' '-e' 'verifyStoredBrand' 'true' 'org.nekosportsworld.tool.tests/org.nekosportsworld.tool.tests.SmokeInstrumentation'
$restartResult
if ($LASTEXITCODE -ne 0 -or ($restartResult -join "`n") -notmatch 'PASS: [0-9]+ Android persistence restart checks') { throw 'Android process restart persistence checks failed' }
