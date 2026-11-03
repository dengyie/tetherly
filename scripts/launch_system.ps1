$code = @'
using System;
using System.Runtime.InteropServices;

public class SessionLauncher {
    [DllImport("kernel32.dll")]
    public static extern uint WTSGetActiveConsoleSessionId();

    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool OpenProcessToken(IntPtr ProcessHandle, uint DesiredAccess, out IntPtr TokenHandle);

    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool DuplicateTokenEx(IntPtr hExistingToken, uint dwDesiredAccess, IntPtr lpTokenAttributes, int ImpersonationLevel, int TokenType, out IntPtr phNewToken);

    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool SetTokenInformation(IntPtr TokenHandle, int TokenInformationClass, ref uint TokenInformation, uint TokenInformationLength);

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern bool CreateProcessAsUser(IntPtr hToken, string lpApplicationName, string lpCommandLine, IntPtr lpProcessAttributes, IntPtr lpThreadAttributes, bool bInheritHandles, uint dwCreationFlags, IntPtr lpEnvironment, string lpCurrentDirectory, ref STARTUPINFO lpStartupInfo, out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll")]
    public static extern IntPtr GetCurrentProcess();

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
    public struct STARTUPINFO {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX; public int dwY; public int dwXSize; public int dwYSize;
        public int dwXCountChars; public int dwYCountChars; public int dwFillAttribute;
        public int dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    public static int LaunchInActiveSession(string cmd) {
        uint sessionId = WTSGetActiveConsoleSessionId();
        if (sessionId == 0xFFFFFFFF) return -1;

        IntPtr hCurrentToken = IntPtr.Zero;
        if (!OpenProcessToken(GetCurrentProcess(), 0x0002 | 0x0008, out hCurrentToken)) {
            return -2;
        }

        IntPtr hNewToken = IntPtr.Zero;
        if (!DuplicateTokenEx(hCurrentToken, 0x02000000, IntPtr.Zero, 2, 1, out hNewToken)) {
            CloseHandle(hCurrentToken);
            return -3;
        }

        // TokenSessionId = 12
        if (!SetTokenInformation(hNewToken, 12, ref sessionId, sizeof(uint))) {
            int err = Marshal.GetLastWin32Error();
            CloseHandle(hNewToken);
            CloseHandle(hCurrentToken);
            return -400 - err;
        }

        STARTUPINFO si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        si.lpDesktop = @"winsta0\default";

        PROCESS_INFORMATION pi = new PROCESS_INFORMATION();
        bool ok = CreateProcessAsUser(hNewToken, null, cmd, IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, null, ref si, out pi);
        int e = Marshal.GetLastWin32Error();

        CloseHandle(hNewToken);
        CloseHandle(hCurrentToken);

        if (!ok) return -500 - e;
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        return pi.dwProcessId;
    }
}
'@

Add-Type -TypeDefinition $code

$cmd = $args[0]
if ([string]::IsNullOrEmpty($cmd)) {
    $cmd = 'cmd.exe /c E:\code\run_tetherly_system.bat'
}

$res = [SessionLauncher]::LaunchInActiveSession($cmd)
"SessionLauncher result: $res" | Out-File -FilePath C:\Users\mango\launcher_result.log
Write-Host "Result: $res"
