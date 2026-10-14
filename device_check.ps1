param(
    [Parameter(Mandatory = $true)][string]$DllPath
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Emit($obj) {
    $obj | ConvertTo-Json -Compress
}

if (-not (Test-Path -LiteralPath $DllPath)) {
    Emit @{ ok = $false; error = "Cable library was not found." }
    exit 0
}

$source = @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public class CableCheck {
    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibrary(string path);

    [DllImport("kernel32", CharSet = CharSet.Ansi, SetLastError = true)]
    public static extern IntPtr GetProcAddress(IntPtr module, string name);

    [DllImport("kernel32", SetLastError = true)]
    public static extern bool FreeLibrary(IntPtr module);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int OpenFn(IntPtr name, out uint deviceId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int CloseFn(uint deviceId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Ansi)]
    public delegate int VersionFn(uint deviceId, StringBuilder firmware, StringBuilder dll, StringBuilder api);

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Ansi)]
    public delegate int ErrorFn(StringBuilder message);

    public static string Run(string path) {
        IntPtr module = LoadLibrary(path);
        if (module == IntPtr.Zero) {
            return "{\"ok\":false,\"error\":\"Windows could not load the cable library.\"}";
        }
        uint deviceId = 0;
        bool opened = false;
        try {
            OpenFn open = (OpenFn)Marshal.GetDelegateForFunctionPointer(Require(module, "PassThruOpen"), typeof(OpenFn));
            CloseFn close = (CloseFn)Marshal.GetDelegateForFunctionPointer(Require(module, "PassThruClose"), typeof(CloseFn));
            VersionFn version = (VersionFn)Marshal.GetDelegateForFunctionPointer(Require(module, "PassThruReadVersion"), typeof(VersionFn));
            ErrorFn lastError = (ErrorFn)Marshal.GetDelegateForFunctionPointer(Require(module, "PassThruGetLastError"), typeof(ErrorFn));
            int status = open(IntPtr.Zero, out deviceId);
            if (status != 0) {
                return "{\"ok\":false,\"error\":" + Json(Describe(lastError, status)) + "}";
            }
            opened = true;
            StringBuilder firmware = new StringBuilder(80);
            StringBuilder dll = new StringBuilder(80);
            StringBuilder api = new StringBuilder(80);
            int versionStatus = version(deviceId, firmware, dll, api);
            close(deviceId);
            opened = false;
            if (versionStatus != 0) {
                return "{\"ok\":false,\"error\":" + Json(Describe(lastError, versionStatus)) + "}";
            }
            return "{\"ok\":true,\"firmware\":" + Json(firmware.ToString()) + ",\"dll\":" + Json(dll.ToString()) + ",\"api\":" + Json(api.ToString()) + "}";
        } catch (Exception ex) {
            if (opened) {
                try {
                    CloseFn close = (CloseFn)Marshal.GetDelegateForFunctionPointer(Require(module, "PassThruClose"), typeof(CloseFn));
                    close(deviceId);
                } catch { }
            }
            return "{\"ok\":false,\"error\":" + Json(ex.Message) + "}";
        } finally {
            FreeLibrary(module);
        }
    }

    static IntPtr Require(IntPtr module, string name) {
        IntPtr address = GetProcAddress(module, name);
        if (address == IntPtr.Zero) throw new Exception("The cable library is missing " + name + ".");
        return address;
    }

    static string Describe(ErrorFn lastError, int status) {
        StringBuilder message = new StringBuilder(80);
        if (lastError(message) == 0 && message.Length > 0) return message.ToString();
        return "The cable library returned status " + status + ".";
    }

    static string Json(string value) {
        return "\"" + (value ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " ") + "\"";
    }
}
"@

Add-Type -TypeDefinition $source -Language CSharp
[CableCheck]::Run($DllPath)
exit 0
