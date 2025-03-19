use std::{io::Read, os::unix::net::UnixStream};

pub struct Connection {
    stream: UnixStream,
}

impl Connection {
    pub fn new() -> anyhow::Result<Self> {
        let sock_path = std::env::var("SWAYSOCK")?;

        let sock = UnixStream::connect(sock_path)?;

        Ok(Self { stream: sock })
    }

    pub fn read_message(&mut self) -> anyhow::Result<()> {
        let mut header_buf = [0u8; size_of_val("i3-ipc") + size_of::<u32>() * 2];

        self.stream.read_exact(header_buf.as_mut_slice())?;

        println!("header_buf: {:?}", header_buf);

        Ok(())
    }
}
