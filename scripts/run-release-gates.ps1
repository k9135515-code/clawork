param(
    [switch]$SkipReleaseBuild,
    [string]$ReportPrefix = "local"
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$preflightPath = "reports/preflight-$ReportPrefix-$timestamp.md"
$readinessPath = "reports/release-readiness-$ReportPrefix-$timestamp.md"

Write-Host "Running preflight..."
$preflightArgs = @(
    "-ExecutionPolicy", "Bypass",
    "-File", "./scripts/preflight-release.ps1",
    "-ReportPath", $preflightPath
)
if ($SkipReleaseBuild) {
    $preflightArgs += "-SkipReleaseBuild"
}
powershell @preflightArgs

Write-Host "Running release readiness..."
powershell -ExecutionPolicy Bypass -File "./scripts/release-readiness.ps1" `
    -PreflightReportPath $preflightPath `
    -ReportPath $readinessPath

Write-Host "Release gates passed."
Write-Host "Preflight report: $preflightPath"
Write-Host "Readiness report: $readinessPath"
