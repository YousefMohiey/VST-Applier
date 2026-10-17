$ErrorActionPreference = 'Continue'
$stage = 'C:\Users\Administrator\vst-applier-rs\scratch\upd_dryrun\stage'
$dir = 'C:\Users\Administrator\vst-applier-rs\scratch\upd_dryrun\app'
$appPid = 999999
$log = Join-Path $stage 'install.log'
"installer start $(Get-Date -Format 'HH:mm:ss') stage=$stage dir=$dir pid=$appPid" | Out-File -FilePath $log -Encoding ascii
# Wait for the app to exit (it holds the exe lock).
while (Get-Process -Id $appPid -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 400 }
"app exited, copying" | Out-File -FilePath $log -Append -Encoding ascii
$files = @('vst-applier.exe', 'Vst2HostNative.dll', 'Vst3HostNative.dll')
$pluginSrc = Join-Path $stage 'Yousef Noise Suppression.dll'
$pluginMoved = -not (Test-Path $pluginSrc)
$ok = $true
foreach ($f in $files) {
    try {
        Copy-Item -Force -ErrorAction Stop (Join-Path $stage $f) (Join-Path $dir $f)
    } catch {
        $ok = $false
        "copy failed for $f : $_" | Out-File -FilePath $log -Append -Encoding ascii
    }
}
if (-not $pluginMoved) {
    try {
        $pdir = Join-Path $dir 'Plugins\VST-Applier'
        New-Item -ItemType Directory -Force -Path $pdir | Out-Null
        Copy-Item -Force -ErrorAction Stop $pluginSrc (Join-Path $pdir 'Yousef Noise Suppression.dll')
        $pluginMoved = $true
        "plugin copied" | Out-File -FilePath $log -Append -Encoding ascii
    } catch {
        $ok = $false
        "copy failed for the bundled plugin: $_" | Out-File -FilePath $log -Append -Encoding ascii
    }
}
if (-not $ok) {
    # Program Files needs the admin token: one elevated retry (UAC prompt).
    "retrying elevated (UAC prompt)..." | Out-File -FilePath $log -Append -Encoding ascii
    $inner = "`$ErrorActionPreference='Stop'; foreach (`$f in @('vst-applier.exe','Vst2HostNative.dll','Vst3HostNative.dll')) { Copy-Item -Force (Join-Path '$stage' `$f) (Join-Path '$dir' `$f) }; if (Test-Path (Join-Path '$stage' 'Yousef Noise Suppression.dll')) { New-Item -ItemType Directory -Force -Path (Join-Path '$dir' 'Plugins\VST-Applier') | Out-Null; Copy-Item -Force (Join-Path '$stage' 'Yousef Noise Suppression.dll') (Join-Path '$dir' 'Plugins\VST-Applier\Yousef Noise Suppression.dll') }; 'elevated copy ok' | Out-File -FilePath '$log' -Append -Encoding ascii"
    try {
        Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', $inner
    } catch {
        "elevated retry failed: $_" | Out-File -FilePath $log -Append -Encoding ascii
    }
}
$installed = Join-Path $dir 'vst-applier.exe'
"relaunching $installed" | Out-File -FilePath $log -Append -Encoding ascii
Start-Process $installed
"done $(Get-Date -Format 'HH:mm:ss')" | Out-File -FilePath $log -Append -Encoding ascii
