[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$releasePath = Join-Path $PSScriptRoot 'release.ps1'
$parseTokens = $null
$parseErrors = $null
$releaseAst = [System.Management.Automation.Language.Parser]::ParseFile($releasePath, [ref]$parseTokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw ($parseErrors | Out-String) }

$statements = @($releaseAst.EndBlock.Statements)
$stampStart = -1
for ($statementIndex = 0; $statementIndex -lt $statements.Count; $statementIndex++) {
    $statement = $statements[$statementIndex]
    if ($statement -is [System.Management.Automation.Language.AssignmentStatementAst] -and
        $statement.Left -is [System.Management.Automation.Language.VariableExpressionAst] -and
        $statement.Left.VariablePath.UserPath -eq 'updated') {
        $stampStart = $statementIndex
        break
    }
}
if ($stampStart -lt 0 -or $statements[$stampStart + 2] -isnot [System.Management.Automation.Language.IfStatementAst]) {
    throw 'Cannot locate manifest version stamping statements'
}
$stampSource = $statements[$stampStart..($stampStart + 2)].Extent.Text -join "`n"
$stampManifest = [scriptblock]::Create($stampSource)
$originalManifest = Get-Content (Join-Path $PSScriptRoot 'AndroidManifest.xml') -Raw
$androidNamespace = 'http://schemas.android.com/apk/res/android'

foreach ($revision in @(1, 99)) {
    $versionCode = 30000 + $revision
    $versionName = "0.3.0-android.$revision"
    $manifestText = $originalManifest
    . $stampManifest
    [xml]$stampedManifest = $updated
    if ($stampedManifest.manifest.GetAttribute('versionCode', $androidNamespace) -ne "$versionCode" -or
        $stampedManifest.manifest.GetAttribute('versionName', $androidNamespace) -ne $versionName) {
        throw "Incorrect manifest version for revision $revision"
    }
    $manifestText = $updated
    . $stampManifest
    if ($updated -ne $manifestText) { throw 'Repeated version stamping changed the manifest' }
}

$manifestText = '<manifest xmlns:android="http://schemas.android.com/apk/res/android" />'
$failedAsExpected = $false
try { . $stampManifest } catch {
    if ($_.Exception.Message -ne 'AndroidManifest.xml 盖章失败（未匹配到 versionCode/versionName）') { throw }
    $failedAsExpected = $true
}
if (-not $failedAsExpected) { throw 'Missing version attributes did not fail stamping' }

Write-Output 'PASS: release script parses; manifest stamping, repeat stamping, and missing-attribute checks passed'
