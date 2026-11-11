# Run only on an owned CI runner. Never launch the app or touch cluster credentials.
$ErrorActionPreference = 'Stop'
$version = '0.1.3'
$setup = (Resolve-Path "dist/Iceon_${version}_x86-setup.exe").Path
$payload = (Resolve-Path 'target/i686-pc-windows-msvc/release/iceon.exe').Path
$root = Join-Path $env:RUNNER_TEMP "iceon-installer-$([guid]::NewGuid())"
$install = Join-Path $root 'Iceon custom location'
$registry = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Iceon'
$legacy = 'Software\Yanuar\Iceon'
$hive = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry32)
function Has-Key($path) {
    $key = $hive.OpenSubKey($path)
    if ($null -eq $key) { return $false }
    $key.Dispose()
    return $true
}
if ((Has-Key $registry) -or (Has-Key $legacy)) { throw 'Runner already has Iceon registration' }
New-Item -ItemType Directory -Path $install -Force | Out-Null
try {
    # Model the prior per-user NSIS install, including a nondefault location.
    $key = $hive.CreateSubKey($legacy)
    $key.SetValue("", $install)
    $key.Dispose()
    [IO.File]::WriteAllText((Join-Path $install 'iceon.exe'), 'old executable fixture')
    [IO.File]::WriteAllText((Join-Path $install 'keep-user-data.txt'), 'preserve')
    foreach ($arguments in @('/S /UPDATE', '/S')) {
        $process = Start-Process -FilePath $setup -ArgumentList $arguments -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Setup failed: $($process.ExitCode)" }
        $installed = Join-Path $install 'iceon.exe'
        if ((Get-FileHash $installed).Hash -ne (Get-FileHash $payload).Hash) { throw 'Installed payload mismatch' }
        $info = $hive.OpenSubKey($registry)
        if ($null -eq $info) { throw 'Missing registration' }
        if ($info.GetValue("DisplayVersion") -ne $version -or $info.GetValue("InstallLocation") -ne $install) { throw 'Registration mismatch' }
        $info.Dispose()
        if (!(Test-Path (Join-Path ([Environment]::GetFolderPath('Programs')) 'Iceon.lnk'))) { throw 'Missing Start menu shortcut' }
    }
    $uninstall = Join-Path $install 'uninstall.exe'
    # _?= keeps the uninstaller in the foreground so its exit can be checked.
    $process = Start-Process -FilePath $uninstall -ArgumentList "/S _?=$install" -Wait -PassThru
    if ($process.ExitCode -ne 0 -or (Has-Key $registry)) { throw 'Uninstall failed' }
    if (Test-Path (Join-Path $install 'iceon.exe')) { throw 'Executable survived uninstall' }
    if (!(Test-Path (Join-Path $install 'keep-user-data.txt'))) { throw 'Uninstall removed unknown user data' }
} finally {
    $hive.DeleteSubKeyTree($registry, $false)
    $hive.DeleteSubKeyTree($legacy, $false)
    $hive.Dispose()
    Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Output 'Installer upgrade/reinstall/uninstall checks passed.'
