param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

$desktopExe = Join-Path $repoRoot "target\debug\clawork-desktop.exe"
$cliExe = Join-Path $repoRoot "target\debug\clawork.exe"

function Invoke-JsonCli {
    param(
        [string[]]$CmdArgs
    )

    $prevErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $raw = & $cliExe @CmdArgs 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prevErrorAction
    if ($exitCode -ne 0) {
        throw "clawork failed: clawork $($CmdArgs -join ' ') :: $raw"
    }
    return ($raw | ConvertFrom-Json)
}

function Invoke-CliExpectFailure {
    param(
        [string[]]$CmdArgs,
        [string]$Contains
    )

    $prevErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $raw = & $cliExe @CmdArgs 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prevErrorAction
    if ($exitCode -eq 0) {
        throw "expected failure but command succeeded: clawork $($CmdArgs -join ' ')"
    }
    $text = "$raw"
    if ($Contains -and $text -notmatch [Regex]::Escape($Contains)) {
        throw "failure output did not contain expected text '$Contains': $text"
    }
    return $text
}

function Get-InitError {
    param(
        [object]$Status,
        [string]$Name
    )

    if ($null -eq $Status) {
        return $null
    }
    if ($Status.PSObject.Properties.Name -notcontains "init_errors") {
        return $null
    }
    $errors = $Status.init_errors
    if ($null -eq $errors) {
        return $null
    }
    if ($errors.PSObject.Properties.Name -contains $Name) {
        return [string]$errors.$Name
    }
    return $null
}

if (-not $SkipBuild) {
    cargo build -p clawork-desktop -p clawork-cli | Out-Null
}

if (-not (Test-Path $desktopExe)) {
    throw "missing desktop binary: $desktopExe"
}
if (-not (Test-Path $cliExe)) {
    throw "missing cli binary: $cliExe"
}

$env:CLAWORK_DAEMON_ONLY = "1"
$desktopProc = $null

try {
    $desktopProc = Start-Process -FilePath $desktopExe -WorkingDirectory $repoRoot -PassThru

    $ready = $false
    for ($i = 0; $i -lt 90; $i++) {
        & $cliExe status > $null 2>&1
        if ($LASTEXITCODE -eq 0) {
            $ready = $true
            break
        }
        Start-Sleep -Seconds 1
    }
    if (-not $ready) {
        throw "local API did not become ready in time"
    }

    $status = Invoke-JsonCli -CmdArgs @("status")
    if (-not $status.daemon_running) {
        # CI can occasionally race daemon startup. Heal by explicitly starting daemon once.
        $null = Invoke-JsonCli -CmdArgs @("daemon", "start")
        Start-Sleep -Milliseconds 800
        $status = Invoke-JsonCli -CmdArgs @("status")
        if (-not $status.daemon_running) {
            throw "expected daemon_running=true after initial start recovery"
        }
    }
    $operatorInitError = Get-InitError -Status $status -Name "operator"
    $memoryInitError = Get-InitError -Status $status -Name "memory"

    $stopRes = Invoke-JsonCli -CmdArgs @("daemon", "stop")
    if (-not $stopRes.ok) {
        throw "daemon stop did not return ok"
    }
    Start-Sleep -Milliseconds 800
    $statusAfterStop = Invoke-JsonCli -CmdArgs @("status")
    if ($statusAfterStop.daemon_running) {
        throw "expected daemon_running=false after stop"
    }

    $startRes = Invoke-JsonCli -CmdArgs @("daemon", "start")
    if (-not $startRes.ok) {
        throw "daemon start did not return ok"
    }
    Start-Sleep -Milliseconds 800
    $statusAfterStart = Invoke-JsonCli -CmdArgs @("status")
    if (-not $statusAfterStart.daemon_running) {
        throw "expected daemon_running=true after start"
    }

    $taskRunRes = Invoke-JsonCli -CmdArgs @("task", "run", "heartbeat")
    if (-not $taskRunRes.ok) {
        throw "task run heartbeat did not return ok"
    }

    # Approval flow: fs write requires confirmation in sandbox; CLI auto-approve + retry.
    $probePath = "data/smoke-e2e.txt"
    $writeRes = Invoke-JsonCli -CmdArgs @("fs", "write", $probePath, "smoke-e2e")
    if (-not $writeRes.ok) {
        throw "fs write did not return ok"
    }
    $readRes = Invoke-JsonCli -CmdArgs @("fs", "read", $probePath)
    if (-not $readRes.ok -or $readRes.content -ne "smoke-e2e") {
        throw "fs read content mismatch after write"
    }

    # Operator flow (skip if operator store is unavailable in this environment).
    $operatorReady = [string]::IsNullOrWhiteSpace($operatorInitError)
    if ($operatorReady) {
        try {
            $session = Invoke-JsonCli -CmdArgs @("operator", "session", "create", "Smoke Session", "Verify operator flow")
            if (-not $session.id) {
                throw "operator session create did not return id"
            }
            $plan = Invoke-JsonCli -CmdArgs @("operator", "session", "plan", $session.id, '["collect","execute","review"]')
            if (-not $plan.id) {
                throw "operator session plan did not return session"
            }
            $run = Invoke-JsonCli -CmdArgs @("operator", "session", "run", $session.id)
            if (-not $run.id) {
                throw "operator session run did not return session"
            }
            $timeline = Invoke-JsonCli -CmdArgs @("operator", "session", "timeline", $session.id, "20")
            if ($timeline.Count -lt 1) {
                throw "operator timeline should have at least one item"
            }
        }
        catch {
            if ($_.Exception.Message -match "not_configured: operator store not initialized") {
                $operatorReady = $false
                Write-Host "Operator flow skipped: operator store not initialized"
            }
            else {
                throw
            }
        }
    } else {
        Write-Host "Operator flow skipped: init error detected in status"
    }

    # Memory flow (skip if memory store is unavailable in this environment).
    $memoryReady = [string]::IsNullOrWhiteSpace($memoryInitError)
    if ($memoryReady) {
        try {
            $memoryProbe = "smoke-memory-" + [guid]::NewGuid().ToString("N")
            $memStore = Invoke-JsonCli -CmdArgs @("memory", "store", $memoryProbe)
            if (-not $memStore) {
                throw "memory store did not return id"
            }
            $memSearch = Invoke-JsonCli -CmdArgs @("memory", "search", $memoryProbe, "5")
            if ($memSearch.Count -lt 1) {
                throw "memory search should return at least one hit"
            }
            $memRecent = Invoke-JsonCli -CmdArgs @("memory", "recent", "5")
            if ($memRecent.Count -lt 1) {
                throw "memory recent should return at least one record"
            }
        }
        catch {
            if ($_.Exception.Message -match "not_configured: memory store not initialized") {
                $memoryReady = $false
                Write-Host "Memory flow skipped: memory store not initialized"
            }
            else {
                throw
            }
        }
    } else {
        Write-Host "Memory flow skipped: init error detected in status"
    }

    # Briefing flow and rationale validation.
    $briefing = Invoke-JsonCli -CmdArgs @("briefing", "show")
    if (-not $briefing.overview) {
        throw "briefing response is missing overview"
    }
    if (-not $briefing.rationale_sources -or $briefing.rationale_sources.Count -lt 1) {
        throw "briefing response is missing rationale_sources"
    }
    $suggestions = Invoke-JsonCli -CmdArgs @("briefing", "suggestions", "10")
    if ($null -eq $suggestions) {
        throw "briefing suggestions response was null"
    }

    # MCP failure path: remote route without URL should fail. Error detail can vary by env.
    $mcpFailure = Invoke-CliExpectFailure -CmdArgs @("mcp", "tools/list", "{}", "--route", "remote") -Contains ""
    if ($mcpFailure -notmatch "not_configured|timeout|api error") {
        throw "unexpected mcp remote failure output: $mcpFailure"
    }

    $operatorStatusText = if ($operatorReady) { "enabled" } else { "skipped-not-configured" }
    $memoryStatusText = if ($memoryReady) { "enabled" } else { "skipped-not-configured" }
    Write-Host "Smoke test passed: local API + daemon + CLI flows (operator=$operatorStatusText, memory=$memoryStatusText)"
}
finally {
    if ($desktopProc -and -not $desktopProc.HasExited) {
        Stop-Process -Id $desktopProc.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item Env:CLAWORK_DAEMON_ONLY -ErrorAction SilentlyContinue
}
