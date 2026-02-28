param(
    [switch]$SkipReleaseBuild,
    [string]$ReportPath = "reports/preflight-latest.md",
    [double]$MaxCliSizeMb = 15
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

$startedAt = Get-Date
$results = [System.Collections.Generic.List[object]]::new()
$overall = "passed"
$failureMessage = $null
$releaseSizeMb = $null

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

function Read-Version {
    param(
        [scriptblock]$Command
    )

    try {
        $output = & $Command
        if ($LASTEXITCODE -ne 0) {
            return "n/a"
        }
        return (($output | Out-String).Trim())
    }
    catch {
        return "n/a"
    }
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

function Write-Report {
    param(
        [string]$OutputPath,
        [DateTime]$StartedAt,
        [DateTime]$FinishedAt,
        [string]$Overall,
        [string]$FailureMessage,
        [System.Collections.Generic.List[object]]$Results,
        [object]$ReleaseSizeMb,
        [double]$MaxCliSizeMb
    )

    $totalDurationSec = [Math]::Round(($FinishedAt - $StartedAt).TotalSeconds, 2)
    $rustcVersion = Read-Version { rustc --version }
    $cargoVersion = Read-Version { cargo --version }
    $nodeVersion = Read-Version { node -v }
    $node22Version = Read-Version { npx -y node@22 -v }

    $releaseSizeText = if ($null -ne $ReleaseSizeMb -and -not [string]::IsNullOrWhiteSpace("$ReleaseSizeMb")) {
        "$ReleaseSizeMb MB"
    }
    else {
        "n/a"
    }

    $lines = @(
        "# Clawork Preflight Report",
        "",
        "- Started at: $($StartedAt.ToString("o"))",
        "- Finished at: $($FinishedAt.ToString("o"))",
        "- Overall: $Overall",
        "- Total duration: $totalDurationSec sec",
        "- rustc: $rustcVersion",
        "- cargo: $cargoVersion",
        "- node: $nodeVersion",
        "- node@22: $node22Version",
        "- Release CLI size budget: <= $MaxCliSizeMb MB",
        "- Release CLI size: $releaseSizeText",
        ""
    )

    if ($FailureMessage) {
        $lines += "- Failure: $(Escape-Markdown $FailureMessage)"
        $lines += ""
    }

    $lines += "| Step | Status | Duration(sec) | Error |"
    $lines += "|---|---|---:|---|"
    foreach ($step in $Results) {
        $lines += "| $(Escape-Markdown $step.Name) | $($step.Status) | $($step.DurationSec) | $(Escape-Markdown $step.Error) |"
    }

    $resolvedOutputPath = Resolve-OutputPath -Path $OutputPath -BaseDir $repoRoot
    $parent = Split-Path $resolvedOutputPath -Parent
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    Set-Content -Path $resolvedOutputPath -Value ($lines -join "`n") -Encoding UTF8
    Write-Host "Report written: $resolvedOutputPath"
}

function Invoke-Step {
    param(
        [string]$Name,
        [scriptblock]$Action
    )

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host "==> $Name"
    try {
        & $Action
        if ($LASTEXITCODE -ne 0) {
            throw "command failed with exit code $LASTEXITCODE"
        }
        $watch.Stop()
        $results.Add([pscustomobject]@{
                Name        = $Name
                Status      = "passed"
                DurationSec = [Math]::Round($watch.Elapsed.TotalSeconds, 2)
                Error       = ""
            })
        Write-Host "OK: $Name"
    }
    catch {
        $watch.Stop()
        $message = $_.Exception.Message
        $results.Add([pscustomobject]@{
                Name        = $Name
                Status      = "failed"
                DurationSec = [Math]::Round($watch.Elapsed.TotalSeconds, 2)
                Error       = $message
            })
        Write-Host "FAILED: $Name"
        throw
    }
}

try {
    Invoke-Step "cargo fmt --all --check" { cargo fmt --all --check }
    Invoke-Step "cargo clippy --workspace --all-targets -- -D warnings" { cargo clippy --workspace --all-targets -- -D warnings }
    Invoke-Step "cargo test --workspace" { cargo test --workspace }
    Invoke-Step "UI build (Node 22)" {
        Push-Location "apps/desktop/ui"
        try {
            npx -y node@22 "C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js" run build
        }
        finally {
            Pop-Location
        }
    }
    Invoke-Step "local smoke (daemon/API/CLI)" { powershell -ExecutionPolicy Bypass -File "./scripts/smoke-local-api.ps1" }

    if (-not $SkipReleaseBuild) {
        Invoke-Step "cargo build --release -p clawork-cli" { cargo build --release -p clawork-cli }
        $cliPath = Join-Path $repoRoot "target\release\clawork.exe"
        if (-not (Test-Path $cliPath)) {
            throw "missing release binary: $cliPath"
        }
        $releaseSizeMb = [Math]::Round(((Get-Item $cliPath).Length / 1MB), 2)
        Write-Host "Release CLI size: $releaseSizeMb MB ($cliPath)"
        Invoke-Step "release size budget (<= $MaxCliSizeMb MB)" {
            if ($releaseSizeMb -gt $MaxCliSizeMb) {
                throw "release CLI exceeds budget: $releaseSizeMb MB > $MaxCliSizeMb MB"
            }
        }
    }

    Write-Host "Preflight release checks passed."
}
catch {
    $overall = "failed"
    $failureMessage = $_.Exception.Message
    Write-Host "Preflight release checks failed: $failureMessage"
}
finally {
    $finishedAt = Get-Date
    Write-Report `
        -OutputPath $ReportPath `
        -StartedAt $startedAt `
        -FinishedAt $finishedAt `
        -Overall $overall `
        -FailureMessage $failureMessage `
        -Results $results `
        -ReleaseSizeMb $releaseSizeMb `
        -MaxCliSizeMb $MaxCliSizeMb
}

if ($overall -eq "failed") {
    throw "preflight failed: $failureMessage"
}
