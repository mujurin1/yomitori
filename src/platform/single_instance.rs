use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::core::HSTRING;

/// 名前付き Mutex による単一起動。drop で手放す
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// 既に別のプロセスが持っていれば None
    pub fn acquire(name: &str) -> windows::core::Result<Option<Self>> {
        let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(name)) }?;
        // CreateMutexW は既にある Mutex でも成功し、最後のエラーで知らせる
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}
