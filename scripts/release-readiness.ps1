param(
    [string]$PreflightReportPath = "reports/preflight-latest.md",
    [string]$ReportPath = "reports/release-readiness-latest.md"
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

function Resolve-OutputPath {
    param(
        [string]$Path,
        [string]$BaseDir
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return Join-Path $BaseDir $Path
}

function Escape-Markdown {
    param(
        [string]$Value
    )
    if ($null -eq $Value) {
        return ""
    }
    return $Value.Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
}

function New-CheckResult {
    param(
        [string]$Name,
        [bool]$Passed,
        [string]$Details
    )

    return [pscustomobject]@{
        Name    = $Name
        Passed  = $Passed
        Details = $Details
    }
}

$preflightPath = Resolve-OutputPath -Path $PreflightReportPath -BaseDir $repoRoot
$reportPath = Resolve-OutputPath -Path $ReportPath -BaseDir $repoRoot

$checks = [System.Collections.Generic.List[object]]::new()

if (Test-Path $preflightPath) {
    $preflightRaw = Get-Content -Path $preflightPath -Raw -Encoding UTF8
    $passed = $preflightRaw -match "(?m)^- Overall: passed$"
    $checks.Add((New-CheckResult -Name "Preflight report exists and passed" -Passed $passed -Details $preflightPath))
}
else {
    $checks.Add((New-CheckResult -Name "Preflight report exists and passed" -Passed $false -Details "missing: $preflightPath"))
}

$releaseWorkflowPath = Join-Path $repoRoot ".github/workflows/release.yml"
if (Test-Path $releaseWorkflowPath) {
    $releaseWorkflow = Get-Content -Path $releaseWorkflowPath -Raw -Encoding UTF8
    $checks.Add((New-CheckResult -Name "Release workflow has preflight gate" -Passed ($releaseWorkflow -match "preflight-gate") -Details ".github/workflows/release.yml"))
    $checks.Add((New-CheckResult -Name "Release workflow generates checksum files" -Passed ($releaseWorkflow -match "sha256") -Details ".github/workflows/release.yml"))
    $checks.Add((New-CheckResult -Name "Release workflow publishes SHA256SUMS" -Passed ($releaseWorkflow -match "SHA256SUMS\.txt") -Details ".github/workflows/release.yml"))
    $checks.Add((New-CheckResult -Name "Release workflow validates 15MB CLI budget" -Passed ($releaseWorkflow -match "Validate Windows release CLI size budget") -Details ".github/workflows/release.yml"))
    $checks.Add((New-CheckResult -Name "Release workflow verifies bundle completeness" -Passed ($releaseWorkflow -match "Verify release bundle completeness") -Details ".github/workflows/release.yml"))
    $checks.Add((New-CheckResult -Name "Release workflow verifies governance reports" -Passed ($releaseWorkflow -match "Verify governance reports") -Details ".github/workflows/release.yml"))
}
else {
    $checks.Add((New-CheckResult -Name "Release workflow exists" -Passed $false -Details "missing: .github/workflows/release.yml"))
}

$installShPath = Join-Path $repoRoot "scripts/install.sh"
if (Test-Path $installShPath) {
    $installSh = Get-Content -Path $installShPath -Raw -Encoding UTF8
    $checks.Add((New-CheckResult -Name "install.sh verifies checksum" -Passed ($installSh -match "CLAWORK_SKIP_CHECKSUM" -and $installSh -match "sha256") -Details "scripts/install.sh"))
}
else {
    $checks.Add((New-CheckResult -Name "install.sh exists" -Passed $false -Details "missing: scripts/install.sh"))
}

$installPsPath = Join-Path $repoRoot "scripts/install.ps1"
if (Test-Path $installPsPath) {
    $installPs = Get-Content -Path $installPsPath -Raw -Encoding UTF8
    $checks.Add((New-CheckResult -Name "install.ps1 verifies checksum" -Passed ($installPs -match "CLAWORK_SKIP_CHECKSUM" -and $installPs -match "Get-FileHash") -Details "scripts/install.ps1"))
}
else {
    $checks.Add((New-CheckResult -Name "install.ps1 exists" -Passed $false -Details "missing: scripts/install.ps1"))
}

$readmePath = Join-Path $repoRoot "README.md"
if (Test-Path $readmePath) {
    $readme = Get-Content -Path $readmePath -Raw -Encoding UTF8
    $checks.Add((New-CheckResult -Name "README documents preflight command" -Passed ($readme -match "preflight-release\.ps1") -Details "README.md"))
    $checks.Add((New-CheckResult -Name "README documents checksum verification" -Passed ($readme -match "CLAWORK_SKIP_CHECKSUM") -Details "README.md"))
}
else {
    $checks.Add((New-CheckResult -Name "README exists" -Passed $false -Details "missing: README.md"))
}

$failed = @($checks | Where-Object { -not $_.Passed })
$overall = if ($failed.Count -eq 0) { "ready" } else { "not_ready" }
$generatedAt = Get-Date

$lines = @(
    "# Clawork Release Readiness",
    "",
    "- Generated at: $($generatedAt.ToString("o"))",
    "- Overall: $overall",
    "- Preflight source: $preflightPath",
    "",
    "| Check | Status | Details |",
    "|---|---|---|"
)

foreach ($check in $checks) {
    $status = if ($check.Passed) { "pass" } else { "fail" }
    $lines += "| $(Escape-Markdown $check.Name) | $status | $(Escape-Markdown $check.Details) |"
}

$parent = Split-Path $reportPath -Parent
if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
}
Set-Content -Path $reportPath -Value ($lines -join "`n") -Encoding UTF8
Write-Host "Readiness report written: $reportPath"

if ($overall -ne "ready") {
    throw "release readiness failed: $($failed.Count) check(s) failed"
}
