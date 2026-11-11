# Updates an existing Lighthouse install on Windows to the latest release, for installs that can't
# update themselves (an older build, or update checks turned off).
#
#   powershell -ExecutionPolicy Bypass -File update-windows.ps1
#   powershell -ExecutionPolicy Bypass -File update-windows.ps1 -Folder "D:\Lighthouse" -Channel bleeding-edge
#   powershell -ExecutionPolicy Bypass -File update-windows.ps1 -EnableAutoUpdates
#
# Only the program (lighthouse.exe) is replaced. Everything else that belongs to the install
# (lighthouse.toml, the database and its journal files, the saved post ids, logs, scripts, any other
# file in the folder) is kept exactly as it is:
#   1. Lighthouse is stopped, so the database is consistent.
#   2. Every file of the install is copied to backups\update-<time>\ with a checksum list.
#   3. lighthouse.exe is replaced (after checking the download against the release's SHA256SUMS).
#   4. Every file is checked against the checksum list; anything missing or changed is put back
#      from the backup, and the update is reported as failed.
#   5. Lighthouse is started again. The backup stays (the newest 3 are kept), in case the new
#      version's first start changes the database and you want to go back.
# Settings are not changed, unless you pass -EnableAutoUpdates (turns on automatic updates in
# lighthouse.toml, after the check above).
#
# Before anything else the updater updates itself: it fetches the current update-windows.ps1 from
# the same release (checked against the release's SHA256SUMS) and, if it differs from this copy,
# runs that one instead with the same options.
param(
    [string]$Folder = "",
    [ValidateSet("stable", "bleeding-edge")][string]$Channel = "stable",
    [string]$Repo = "Hailey-Ross/hails.Lighthouse",
    [switch]$EnableAutoUpdates,
    # Set by the updater when it hands over to a newer copy of itself.
    [switch]$NoSelfUpdate
)
$ErrorActionPreference = "Stop"
$TaskName = "Lighthouse"
$Asset = "lighthouse-x86_64-pc-windows-msvc.exe"
$KeepBackups = 3
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Write-Step($text) { Write-Host "-> $text" -ForegroundColor Cyan }

# ---------- first, make sure this updater is the current one ----------
if (-not $NoSelfUpdate -and $PSCommandPath) {
    $fresh = Join-Path ([IO.Path]::GetTempPath()) ("update-windows-" + [Guid]::NewGuid().ToString("N") + ".ps1")
    $sums = "$fresh.sums"
    try {
        $base = "https://github.com/$Repo/releases/download/$Channel"
        Invoke-WebRequest -UseBasicParsing "$base/update-windows.ps1" -OutFile $fresh
        Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile $sums
        $want = (Get-Content $sums | Where-Object { $_ -match 'update-windows\.ps1$' } | Select-Object -First 1) -split '\s+' |
            Select-Object -First 1
        $got = (Get-FileHash -Algorithm SHA256 $fresh).Hash
        if (-not $want -or $got -ne $want.ToUpper()) { throw "the downloaded updater doesn't match the release checksum" }
        if ($got -ne (Get-FileHash -Algorithm SHA256 -LiteralPath $PSCommandPath).Hash) {
            Write-Step "A newer version of this updater is available; using it"
            $pass = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $fresh, "-Channel", $Channel, "-Repo", $Repo, "-NoSelfUpdate")
            if ($Folder) { $pass += @("-Folder", $Folder) }
            if ($EnableAutoUpdates) { $pass += "-EnableAutoUpdates" }
            & powershell @pass
            $code = $LASTEXITCODE
            Remove-Item -Force $fresh, $sums -ErrorAction SilentlyContinue
            exit $code
        }
        Write-Host "   updater is current"
    } catch {
        Write-Host "   could not check for a newer updater ($($_.Exception.Message)); continuing with this one"
    }
    Remove-Item -Force $fresh, $sums -ErrorAction SilentlyContinue
}

# ---------- find the install ----------
$task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if (-not $Folder) {
    $running = Get-Process lighthouse -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1
    if ($running) {
        $Folder = Split-Path -Parent $running.Path
    } elseif ($task -and $task.Actions[0].WorkingDirectory) {
        $Folder = $task.Actions[0].WorkingDirectory
    } else {
        $Folder = "C:\Lighthouse"
    }
}
$Folder = (Resolve-Path $Folder -ErrorAction SilentlyContinue).Path
$exe = if ($Folder) { Join-Path $Folder "lighthouse.exe" } else { "" }
if (-not $exe -or -not (Test-Path $exe)) {
    throw "No Lighthouse found. Run again with -Folder <the folder that holds lighthouse.exe>."
}
Write-Step "Updating the Lighthouse in $Folder"

# Stopping a task that runs as SYSTEM, and reading its protected settings, need administrator rights.
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if ($task -and -not $admin) {
    Write-Step "Asking for administrator rights (the Lighthouse task has to be stopped and started)"
    $args = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Folder", "`"$Folder`"",
              "-Channel", $Channel, "-Repo", $Repo, "-NoSelfUpdate")
    if ($EnableAutoUpdates) { $args += "-EnableAutoUpdates" }
    Start-Process powershell -Verb RunAs -ArgumentList $args -Wait
    exit
}
Write-Host "   now: $((& $exe --version 2>$null) -join ' ')"

# The install's own files: everything in the folder except the program and the backups.
function Get-InstallFiles {
    Get-ChildItem -LiteralPath $Folder -Recurse -File -Force | Where-Object {
        $rel = $_.FullName.Substring($Folder.Length).TrimStart('\')
        $rel -ne "lighthouse.exe" -and $rel -ne "lighthouse.exe.old" -and -not $rel.StartsWith("backups\")
    }
}
function Get-Relative($full) { $full.Substring($Folder.Length).TrimStart('\') }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("lighthouse-update-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory $tmp | Out-Null
$stopped = $false
try {
    # ---------- download and verify (before touching anything) ----------
    $base = "https://github.com/$Repo/releases/download/$Channel"
    Write-Step "Downloading the $Channel build"
    Invoke-WebRequest -UseBasicParsing "$base/$Asset" -OutFile "$tmp\$Asset"
    Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS"
    $want = (Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape($Asset) + '$' } |
        Select-Object -First 1) -split '\s+' | Select-Object -First 1
    $got = (Get-FileHash -Algorithm SHA256 "$tmp\$Asset").Hash.ToLower()
    if (-not $want -or $got -ne $want.ToLower()) { throw "The download doesn't match the release checksum; nothing was changed." }
    Write-Host "   checksum ok"

    # ---------- stop ----------
    Write-Step "Stopping Lighthouse"
    if ($task) { Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue }
    Get-Process lighthouse -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
    $stopped = $true
    for ($i = 0; $i -lt 20 -and (Get-Process lighthouse -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }); $i++) {
        Start-Sleep -Milliseconds 500
    }

    # ---------- back up the install's files ----------
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $backup = Join-Path $Folder "backups\update-$stamp"
    Write-Step "Backing up settings and data to backups\update-$stamp"
    $manifest = @{}
    foreach ($f in Get-InstallFiles) {
        $rel = Get-Relative $f.FullName
        $dest = Join-Path $backup $rel
        New-Item -ItemType Directory -Force (Split-Path -Parent $dest) | Out-Null
        Copy-Item -LiteralPath $f.FullName -Destination $dest -Force
        $manifest[$rel] = (Get-FileHash -Algorithm SHA256 -LiteralPath $f.FullName).Hash
    }
    Copy-Item -LiteralPath $exe -Destination (Join-Path $backup "lighthouse.exe")
    $manifest.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Value)  $($_.Name)" } |
        Set-Content -Encoding utf8 (Join-Path $backup "SHA256SUMS.txt")
    Write-Host "   $($manifest.Count) file(s) backed up"

    # ---------- swap the program ----------
    Write-Step "Replacing lighthouse.exe"
    Copy-Item -Force $exe "$exe.old"
    Copy-Item -Force "$tmp\$Asset" $exe

    # ---------- check every file came through ----------
    Write-Step "Checking settings and data"
    $problems = @()
    foreach ($rel in $manifest.Keys) {
        $path = Join-Path $Folder $rel
        $ok = (Test-Path -LiteralPath $path) -and ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash -eq $manifest[$rel])
        if (-not $ok) {
            $problems += $rel
            New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
            Copy-Item -LiteralPath (Join-Path $backup $rel) -Destination $path -Force
        }
    }
    if ($problems) {
        throw "These files changed during the update and were put back from the backup: $($problems -join ', '). Check them before starting Lighthouse."
    }
    Write-Host "   all $($manifest.Count) file(s) unchanged"

    # Keep the newest backups only.
    Get-ChildItem (Join-Path $Folder "backups") -Directory -Filter "update-*" | Sort-Object Name -Descending |
        Select-Object -Skip $KeepBackups | Remove-Item -Recurse -Force
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# ---------- optional: let it update itself from now on ----------
$config = Join-Path $Folder "lighthouse.toml"
if ($EnableAutoUpdates -and (Test-Path $config)) {
    $lines = [Collections.Generic.List[string]](Get-Content $config)
    $start = $lines.FindIndex([Predicate[string]] { param($l) $l.Trim() -eq "[updates]" })
    $set = [ordered]@{ check = "true"; auto_install = "true"; channel = "`"$Channel`"" }
    if ($start -lt 0) {
        $lines.Add(""); $lines.Add("[updates]")
        foreach ($k in $set.Keys) { $lines.Add("$k = $($set[$k])") }
    } else {
        $end = $start + 1
        while ($end -lt $lines.Count -and -not $lines[$end].Trim().StartsWith("[")) { $end++ }
        foreach ($k in $set.Keys) {
            $at = -1
            for ($i = $start + 1; $i -lt $end; $i++) { if ($lines[$i] -match "^\s*$k\s*=") { $at = $i } }
            if ($at -ge 0) { $lines[$at] = "$k = $($set[$k])" } else { $lines.Insert($end, "$k = $($set[$k])"); $end++ }
        }
    }
    [IO.File]::WriteAllLines($config, $lines)
    Write-Host "   automatic updates turned on in lighthouse.toml ($Channel); the original is in the backup"
}

# ---------- start ----------
$after = (& $exe --version 2>$null) -join " "
if ($task) {
    Write-Step "Starting Lighthouse"
    Start-ScheduledTask -TaskName $TaskName
} elseif ($stopped) {
    Write-Host "   Lighthouse isn't set up as a Windows task here; start it the way you usually do."
}
Write-Host ""
Write-Host "Done: $after" -ForegroundColor Green
Write-Host "Settings and data are unchanged; a copy is in $Folder\backups. The previous program is kept as lighthouse.exe.old."
