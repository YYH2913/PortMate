param(
    [Parameter(Mandatory = $true)][string]$Manifest,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$dumpbin = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw 'MSVC dumpbin is unavailable for loader diagnostics' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PortMateLoaderProbe {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    public static extern IntPtr LoadLibraryExW(string name, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true, SetLastError = true)]
    public static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    public static extern uint GetModuleFileNameW(IntPtr module, StringBuilder filename, int size);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool FreeLibrary(IntPtr module);
}
'@
$report = [System.Collections.Generic.List[string]]::new()
foreach ($executable in (Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json)) {
    $report.Add("Executable: $executable")
    $lines = & $dumpbin /nologo /imports $executable
    if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for $executable" }
    $lines | Set-Content -LiteralPath (Join-Path $OutputDirectory ((Split-Path -Leaf $executable) + '.imports.log'))
    $imports = @{}
    $current = $null
    foreach ($line in $lines) {
        if ($line -match '^\s+(\S+\.dll)\s*$') {
            $current = $Matches[1]
            if (-not $imports.ContainsKey($current)) { $imports[$current] = [System.Collections.Generic.List[string]]::new() }
        } elseif ($current -and $line -match '^\s+[0-9A-Fa-f]+\s+([A-Za-z_?@$][^\s]*)\s*$') {
            $imports[$current].Add($Matches[1])
        }
    }
    foreach ($dll in ($imports.Keys | Sort-Object)) {
        $candidate = Join-Path (Split-Path -Parent $executable) $dll
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { $candidate = $dll }
        # Map for export inspection only; do not execute imported DLL entrypoints.
        $module = [PortMateLoaderProbe]::LoadLibraryExW($candidate, [IntPtr]::Zero, 1)
        if ($module -eq [IntPtr]::Zero) {
            $report.Add("UNAVAILABLE DLL $dll : Win32 $([Runtime.InteropServices.Marshal]::GetLastWin32Error())")
            continue
        }
        try {
            $path = [Text.StringBuilder]::new(32768)
            [void][PortMateLoaderProbe]::GetModuleFileNameW($module, $path, $path.Capacity)
            $report.Add("DLL $dll => $path")
            foreach ($symbol in $imports[$dll]) {
                if ([PortMateLoaderProbe]::GetProcAddress($module, $symbol) -eq [IntPtr]::Zero) {
                    $report.Add("MISSING ENTRYPOINT ${dll}!$symbol")
                }
            }
        } finally { [void][PortMateLoaderProbe]::FreeLibrary($module) }
    }
}
$report | Set-Content -LiteralPath (Join-Path $OutputDirectory 'loader-report.log')
$report | Write-Output
