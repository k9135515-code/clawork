param(
  [string]$Repo = $env:CLAWORK_REPO,
  [string]$Version = $env:CLAWORK_VERSION,
  [string]$InstallDir = $env:CLAWORK_INSTALL_DIR
)

if ([string]::IsNullOrWhiteSpace($Repo)) { $Repo = "your-org/clawork" }
if ([string]::IsNullOrWhiteSpace($Version)) { $Version = "latest" }
if ([string]::IsNullOrWhiteSpace($InstallDir)) { $InstallDir = "$env:USERPROFILE\\.local\\bin" }

if ($Repo -eq "your-org/clawork") {
  throw "Set CLAWORK_REPO=<owner/repo> before running installer."
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null

if ($Version -eq "latest") {
  $apiUrl = "https://api.github.com/repos/$Repo/releases/latest"
  $release = Invoke-RestMethod -Uri $apiUrl -Method Get
  $tag = $release.tag_name
} else {
  $tag = $Version
}

if ([string]::IsNullOrWhiteSpace($tag)) {
  throw "Failed to resolve release tag"
}

$arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { throw "Unsupported architecture" }
$asset = "clawork-windows-$arch.zip"
$url = "https://github.com/$Repo/releases/download/$tag/$asset"
$checksumUrl = "$url.sha256"

$tmpDir = Join-Path $env:TEMP ("clawork-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

try {
  $zipPath = Join-Path $tmpDir $asset
  $checksumPath = Join-Path $tmpDir "$asset.sha256"
  Invoke-WebRequest -Uri $url -OutFile $zipPath
  if ($env:CLAWORK_SKIP_CHECKSUM -ne "1") {
    Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath
    $checksumLine = Get-Content -Path $checksumPath | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($checksumLine)) {
      throw "Failed to parse checksum from $checksumUrl"
    }
    $expected = ($checksumLine -split '\s+')[0].Trim().ToLowerInvariant()
    $actual = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
      throw "Checksum mismatch for $asset. expected=$expected actual=$actual"
    }
  }
  Expand-Archive -Path $zipPath -DestinationPath $tmpDir -Force
  Copy-Item -Path (Join-Path $tmpDir "clawork.exe") -Destination (Join-Path $InstallDir "clawork.exe") -Force
  Write-Host "Installed clawork to $InstallDir\\clawork.exe"
} finally {
  Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}
