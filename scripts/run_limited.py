"""Run one Windows build/test process tree under a hard commit-memory limit.

The child starts suspended and is resumed only after job assignment succeeds.
Closing this launcher's job handle kills remaining descendants. Concurrent
launchers fail instead of competing for memory. No unguarded fallback exists.

Use the default 768 MiB for script compilation and runtime tests. Native Rust
builds may use up to 3072 MiB; the larger ceiling does not change the default.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import os
from pathlib import Path
import subprocess
import sys
import time


class BasicLimits(c.Structure):
    _fields_ = [("process_time", c.c_int64), ("job_time", c.c_int64),
                ("flags", w.DWORD), ("min_working", c.c_size_t),
                ("max_working", c.c_size_t), ("active", w.DWORD),
                ("affinity", c.c_size_t), ("priority", w.DWORD),
                ("scheduling", w.DWORD)]


class Limits(c.Structure):
    _fields_ = [("basic", BasicLimits), ("io", c.c_uint64 * 6),
                ("process_memory", c.c_size_t), ("job_memory", c.c_size_t),
                ("peak_process", c.c_size_t), ("peak_job", c.c_size_t)]


def run(args):
    if sys.platform != "win32":
        raise RuntimeError("This guard requires Windows job objects")
    import _winapi
    import msvcrt

    kernel = c.WinDLL("kernel32", use_last_error=True)
    signatures = {
        "CreateJobObjectW": ([c.c_void_p, w.LPCWSTR], w.HANDLE),
        "SetInformationJobObject": ([w.HANDLE, c.c_int, c.c_void_p, w.DWORD], w.BOOL),
        "QueryInformationJobObject": ([w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p], w.BOOL),
        "AssignProcessToJobObject": ([w.HANDLE, w.HANDLE], w.BOOL),
        "CreateMutexW": ([c.c_void_p, w.BOOL, w.LPCWSTR], w.HANDLE),
        "ReleaseMutex": ([w.HANDLE], w.BOOL),
        "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
        "ResumeThread": ([w.HANDLE], w.DWORD),
        "TerminateJobObject": ([w.HANDLE, w.UINT], w.BOOL),
        "CloseHandle": ([w.HANDLE], w.BOOL),
    }
    for name, (parameters, result) in signatures.items():
        function = getattr(kernel, name)
        function.argtypes, function.restype = parameters, result

    def checked(ok):
        if not ok:
            raise c.WinError(c.get_last_error())
        return ok

    mutex = checked(kernel.CreateMutexW(None, False, "Local\\SplitScriptResourceGuard"))
    owned = kernel.WaitForSingleObject(mutex, 0) in (0, 128)
    if not owned:
        kernel.CloseHandle(mutex)
        raise RuntimeError("Another guarded build/test is already running")
    job = process = thread = None
    try:
        job = checked(kernel.CreateJobObjectW(None, None))
        limits = Limits()
        # Hard per-process and aggregate commit limits; deny child breakaway.
        limits.basic.flags = 0x100 | 0x200 | 0x2000 | 0x400
        limits.process_memory = limits.job_memory = args.memory_mib * 1024 * 1024
        checked(kernel.SetInformationJobObject(job, 9, c.byref(limits), c.sizeof(limits)))
        installed = Limits()
        checked(kernel.QueryInformationJobObject(job, 9, c.byref(installed), c.sizeof(installed), None))
        if installed.basic.flags != limits.basic.flags or installed.job_memory != limits.job_memory or installed.process_memory != limits.process_memory:
            raise RuntimeError("The requested hard memory limits were not installed")

        log = Path(args.log).resolve()
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("wb") as output, open(os.devnull, "rb") as null:
            startup = subprocess.STARTUPINFO()
            startup.dwFlags = subprocess.STARTF_USESTDHANDLES
            startup.hStdInput = msvcrt.get_osfhandle(null.fileno())
            startup.hStdOutput = startup.hStdError = msvcrt.get_osfhandle(output.fileno())
            os.set_handle_inheritable(startup.hStdInput, True)
            os.set_handle_inheritable(startup.hStdOutput, True)
            environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0", RUST_TEST_THREADS="1")
            process, thread, pid, _ = _winapi.CreateProcess(
                None, subprocess.list2cmdline(args.command), None, None, True,
                0x4 | 0x08000000 | 0x4000, environment, None, startup)
            try:
                checked(kernel.AssignProcessToJobObject(job, process))
            except BaseException:
                _winapi.TerminateProcess(process, 1)
                raise
            if kernel.ResumeThread(thread) == 0xFFFFFFFF:
                raise c.WinError(c.get_last_error())
            print(f"Guard active: PID {pid}, process tree <= {args.memory_mib} MiB, timeout {args.seconds}s", flush=True)
            deadline = time.monotonic() + args.seconds
            while True:
                wait = kernel.WaitForSingleObject(process, 100)
                if wait == 0:
                    break
                if wait != 258:
                    raise c.WinError(c.get_last_error())
                if time.monotonic() >= deadline:
                    checked(kernel.TerminateJobObject(job, 124))
                    break
            kernel.WaitForSingleObject(process, 5000)
            code = _winapi.GetExitCodeProcess(process)
            measured = Limits()
            checked(kernel.QueryInformationJobObject(job, 9, c.byref(measured), c.sizeof(measured), None))
            print(f"Exit {code}; peak committed job memory {measured.peak_job / 1048576:.1f} MiB; log: {log}", flush=True)
            return code
    finally:
        # This also stops descendants when interrupted or when the parent exits.
        if job:
            kernel.CloseHandle(job)
        for handle in (thread, process):
            if handle:
                kernel.CloseHandle(handle)
        kernel.ReleaseMutex(mutex)
        kernel.CloseHandle(mutex)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--memory-mib", type=int, default=768)
    parser.add_argument("--seconds", type=int, default=120)
    parser.add_argument("--log", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command.pop(0)
    if not args.command or not 32 <= args.memory_mib <= 3072 or not 1 <= args.seconds <= 1800:
        parser.error("provide a command, a 32..3072 MiB memory cap, and a 1..1800s timeout")
    sys.exit(run(args))
