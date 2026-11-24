//! Contrôle des processus du moteur : console masquée, pause réelle et arrêt lié à l'application.

use std::process::{Child, Command};

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::OnceLock;

    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process: *mut c_void) -> i32;
        fn NtResumeProcess(process: *mut c_void) -> i32;
    }

    // Le handle reste ouvert toute la vie de l'application : à sa fermeture, même brutale,
    // Windows termine les moteurs rattachés à ce job.
    static JOB: OnceLock<usize> = OnceLock::new();

    fn job() -> *mut c_void {
        *JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            handle as usize
        }) as *mut c_void
    }

    pub fn prepare(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    pub fn adopt(child: &Child) {
        unsafe {
            AssignProcessToJobObject(job(), child.as_raw_handle());
        }
    }

    pub fn suspend(child: &Child) -> bool {
        unsafe { NtSuspendProcess(child.as_raw_handle()) == 0 }
    }

    pub fn resume(child: &Child) -> bool {
        unsafe { NtResumeProcess(child.as_raw_handle()) == 0 }
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    pub fn prepare(command: &mut Command) {
        unsafe {
            command.pre_exec(|| {
                #[cfg(target_os = "linux")]
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }

    pub fn adopt(_child: &Child) {}

    pub fn suspend(child: &Child) -> bool {
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGSTOP) == 0 }
    }

    pub fn resume(child: &Child) -> bool {
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGCONT) == 0 }
    }
}

pub fn prepare(command: &mut Command) {
    imp::prepare(command);
}

pub fn adopt(child: &Child) {
    imp::adopt(child);
}

pub fn suspend(child: &Child) -> bool {
    imp::suspend(child)
}

pub fn resume(child: &Child) -> bool {
    imp::resume(child)
}
