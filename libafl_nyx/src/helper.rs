use libafl::Error;
use libnyx::{NyxConfig, NyxProcess, NyxProcessRole};

pub struct NyxHelper {
    pub nyx_process: NyxProcess,

    pub bitmap_size: usize,
    pub bitmap_buffer: *mut u8,
}

impl NyxHelper {
    pub fn new(
        share_dir: &str,
        cpu_id: usize,
        parent_cpu_id: Option<usize>,
        snap_mode: bool,
    ) -> Result<Self, Error> {
        let mut nyx_config = NyxConfig::load(share_dir)
            .map_err(|e| Error::illegal_argument(format!("Failed to load Nyx config: {e}")))?;
        nyx_config.set_process_role(match parent_cpu_id {
            None => NyxProcessRole::StandAlone,
            Some(id) if id == cpu_id => NyxProcessRole::Parent,
            _ => NyxProcessRole::Child,
        });
        nyx_config.set_worker_id(cpu_id);

        let mut nyx_process = NyxProcess::new(&mut nyx_config, cpu_id)
            .map_err(|e| Error::illegal_argument(format!("Failed to create Nyx process: {e}")))?;
        nyx_process.option_set_reload_mode(snap_mode);
        nyx_process.option_apply();

        let bitmap_size = nyx_process.bitmap_buffer_size();
        let bitmap_buffer = nyx_process.bitmap_buffer_mut().as_mut_ptr();

        Ok(Self {
            nyx_process,
            bitmap_size,
            bitmap_buffer,
        })
    }

    /// Change the timeout for Nyx.
    pub fn set_timeout(&mut self, secs: u8, micro_secs: u32) {
        self.nyx_process.option_set_timeout(secs, micro_secs);
        self.nyx_process.option_apply();
    }
}
