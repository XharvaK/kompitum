# Installs kompitum-core (kom) on Windows, or updates it:
#
#   powershell -c "irm https://releases.kompitum.invalid/core/install.ps1 | iex"
#
# Into %LOCALAPPDATA%\Programs\kompitum-core (kompitum-core.exe, Microsoft's
# ConPTY beside it), added to your PATH, with kom.exe linked to it only if no
# `kom` exists. An kom that is already there is swapped the planned way (kom
# update --from): running terminals keep running. No admin needed.
# KOMPITUM_VERSION=0.2.0 picks a version; KOMPITUM_RELEASES_URL points elsewhere
# (tests); KOMPITUM_INSTALL_DIR installs elsewhere.
#
# `-File install.ps1 <release zip> [session]` installs from a zip already
# here, checked by whoever put it here, and removes both files after
# (`kom machine add` from another machine does that).

# Its own scope: under `irm | iex` it would otherwise set the caller's variables.
& {
$zip, $session = $args
if ($session) { $env:KOMPITUM_SESSION = $session }
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# throw, not exit: under `irm | iex`, exit would close the person's window.
function Fail($why) { throw "kom: $why" }

$releases = if ($env:KOMPITUM_RELEASES_URL) { $env:KOMPITUM_RELEASES_URL.TrimEnd("/") } else { "https://releases.kompitum.invalid" }
$dir = if ($env:KOMPITUM_INSTALL_DIR) { $env:KOMPITUM_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\kompitum-core" }
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne "AMD64") { Fail "no build for $arch Windows yet (x64 only)" }
$target = "windows-x86_64"
# Windows' own tar opens zips; Git Bash's, often first on PATH, can't.
$tar = Join-Path $env:SystemRoot "System32\tar.exe"
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue) -or -not (Test-Path $tar)) {
    Fail "needs curl.exe and tar.exe (Windows 10 1803 or newer)"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) "kom-install-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  if ($zip) {
    Copy-Item $zip "$tmp\kom.zip"
  } else {
    $manifest = if ($env:KOMPITUM_VERSION) { "$releases/core/$($env:KOMPITUM_VERSION)/manifest.json" } else { "$releases/core/latest.json" }
    curl.exe -fsSL $manifest -o "$tmp\manifest.json"
    if ($LASTEXITCODE) { Fail "cannot reach $manifest" }
    $m = Get-Content -Raw "$tmp\manifest.json" | ConvertFrom-Json
    $t = $m.targets.$target
    if (-not $t.url -or -not $t.sha256) { Fail "no build for $target in $manifest" }

    # Tells the release server's download counter a new install from an
    # update; the file is the same either way. Only plain URLs on the release
    # server in use are marked: one with a query or fragment (a mirror's
    # signed URL) is fetched exactly as the manifest gives it.
    $src = if (Test-Path (Join-Path $dir "kompitum-core.exe")) { "update" } else { "install" }
    $plain = $t.url.StartsWith("$releases/", [StringComparison]::Ordinal) -and $t.url.IndexOfAny([char[]]"?#") -lt 0
    $download = if ($plain) { "$($t.url)?src=$src" } else { $t.url }
    curl.exe -fsSL $download -o "$tmp\kom.zip"
    if ($LASTEXITCODE) { Fail "download failed: $($t.url)" }
    # .NET, not Get-FileHash: that one fails when PowerShell 7 starts 5.1.
    $stream = [IO.File]::OpenRead("$tmp\kom.zip")
    try { $got = -join ([Security.Cryptography.SHA256]::Create().ComputeHash($stream) | ForEach-Object { $_.ToString("x2") }) } finally { $stream.Close() }
    if ($got -ne $t.sha256) { Fail "the download is damaged (checksum mismatch); try again" }
  }
    New-Item -ItemType Directory -Force "$tmp\kom" | Out-Null
    & $tar -xf "$tmp\kom.zip" -C "$tmp\kom"
    $new = "$tmp\kom\kompitum-core.exe"
    if ($LASTEXITCODE -or -not (Test-Path "$tmp\kom\conpty\conpty.dll")) { Fail "the download holds no complete kom" }
    $version = (& $new --version) -replace "^kom ", ""
    if ($LASTEXITCODE) { Fail "the downloaded kom does not run on this machine" }

    New-Item -ItemType Directory -Force $dir | Out-Null
    $exe = Join-Path $dir "kompitum-core.exe"
    if (Test-Path $exe) {
        # Swapped the planned way, so its terminals keep running.
        & $exe update --from $new | Out-Null
        if ($LASTEXITCODE) { Fail "updating $exe failed (see above)" }
        Write-Host "kom updated to $version."
    } else {
        Copy-Item -Recurse -Force "$tmp\kom\kompitum-core.exe", "$tmp\kom\kompitum-shell.exe", "$tmp\kom\conpty" $dir
        Write-Host "kom $version installed into $dir."
    }
    Copy-Item -Recurse -Force "$tmp\kom\THIRD-PARTY-NOTICES" $dir -ErrorAction SilentlyContinue
    if (-not (Get-Command kom -ErrorAction SilentlyContinue) -and -not (Test-Path (Join-Path $dir "kom.exe"))) {
        New-Item -ItemType HardLink -Path (Join-Path $dir "kom.exe") -Target $exe | Out-Null
    }

    # The raw user PATH, so %VARIABLES% in it stay unexpanded and keep their type.
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    $raw = [string]$key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $listed = $raw -split ";" | Where-Object { $_.Trim().TrimEnd("\") -ieq $dir.TrimEnd("\") }
    if (-not $listed) {
        $kind = if ($key.GetValueNames() -contains "Path") { $key.GetValueKind("Path") } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
        $key.SetValue("Path", $(if ($raw) { "$($raw.TrimEnd(';'));$dir" } else { $dir }), $kind)
        # Setting any user variable tells running programs the environment changed.
        [Environment]::SetEnvironmentVariable("KOMPITUM_INSTALLING", "1", "User")
        [Environment]::SetEnvironmentVariable("KOMPITUM_INSTALLING", $null, "User")
        Write-Host "Added $dir to your PATH: new terminals have kom."
    }
    $key.Close()
    if (($env:Path -split ";") -notcontains $dir) { $env:Path += ";$dir" }
    Write-Host "Agents: kom --skill. People: kom --help."
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    if ($zip) { Remove-Item -Force $zip, $PSCommandPath -ErrorAction SilentlyContinue }
}
} @args
