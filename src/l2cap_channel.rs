// This code is orginally written by <https://github.com/Dirbaio>.

use std::io::{Read, Write};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{fmt, pin, thread};

use futures_lite::io::{AsyncRead, AsyncWrite, BlockOn};
use jni::{objects::JByteArray, refs::Global};
use log::{debug, trace, warn};

use crate::bindings;
use crate::error::ErrorKind;
use crate::util::{android_api_level, jni_with_env, JByteArrayExt, ReferenceExt};

const PIPE_CAPACITY: usize = 0x100000; // 1MB

macro_rules! derive_async_read {
    ($type:ty, $field:tt) => {
        impl AsyncRead for $type {
            fn poll_read(
                mut self: pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut [u8],
            ) -> Poll<std::io::Result<usize>> {
                let reader = pin::pin!(&mut self.$field);
                reader.poll_read(cx, buf)
            }
        }
    };
}

macro_rules! derive_async_write {
    ($type:ty, $field:tt) => {
        impl AsyncWrite for $type {
            fn poll_write(
                mut self: pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                let writer = pin::pin!(&mut self.$field);
                writer.poll_write(cx, buf)
            }

            fn poll_flush(
                mut self: pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                let writer = pin::pin!(&mut self.$field);
                writer.poll_flush(cx)
            }

            fn poll_close(
                mut self: pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                let writer = pin::pin!(&mut self.$field);
                writer.poll_close(cx)
            }

            fn poll_write_vectored(
                mut self: pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
                bufs: &[std::io::IoSlice<'_>],
            ) -> Poll<std::io::Result<usize>> {
                let writer = pin::pin!(&mut self.$field);
                writer.poll_write_vectored(cx, bufs)
            }
        }
    };
}

pub fn open_l2cap_channel(
    device: Global<bindings::BluetoothDevice<'static>>,
    psm: u16,
    secure: bool,
) -> std::prelude::v1::Result<(L2capChannelReader, L2capChannelWriter), crate::Error> {
    if android_api_level() < 29 {
        return Err(crate::Error::new(
            ErrorKind::NotSupported,
            None,
            "creating L2CAP channel requires Android API level 29 or higher",
        ));
    }
    jni_with_env(|env| {
        let channel = env
            .call_method(
                &device,
                if secure {
                    jni::jni_str!("createL2capChannel")
                } else {
                    jni::jni_str!("createInsecureL2capChannel")
                },
                jni::jni_sig!((jint) -> android.bluetooth.BluetoothSocket),
                &[psm.into()],
            )?
            .l()?;
        let channel = bindings::BluetoothSocket::cast_local(env, channel)?.non_null()?;
        channel.connect(env)?;

        // The L2capCloser closes the l2cap channel when dropped.
        // We put it in an Arc held by both the reader and writer, so it gets dropped
        // when
        let closer = Arc::new(L2capCloser {
            channel: env.new_global_ref(&channel)?,
        });

        let (read_receiver, read_sender) = piper::pipe(PIPE_CAPACITY);
        let (write_receiver, write_sender) = piper::pipe(PIPE_CAPACITY);
        let input_stream = channel.get_input_stream(env)?;
        let output_stream = channel.get_output_stream(env)?;
        let (input_stream, output_stream) = (
            env.new_global_ref(input_stream)?,
            env.new_global_ref(output_stream)?,
        );

        // Unfortunately, Android's API for L2CAP channels is only blocking. Only way to deal with it
        // is to launch two background threads with blocking loops for reading and writing, which communicate
        // with the async Rust world via async channels.
        //
        // The loops stop when either Android returns an error (for example if the channel is closed), or the
        // async channel gets closed because the user dropped the reader or writer structs.
        thread::spawn(move || {
            debug!("l2cap read thread running!");
            let mut read_sender = BlockOn::new(read_sender);

            let _ = jni_with_env(|env| {
                let jarr = JByteArray::new(env, 1024)?;
                loop {
                    match input_stream.read(env, &jarr) {
                        Ok(n) if n < 0 => {
                            warn!("failed to read from l2cap channel: {}", n);
                            break;
                        }
                        Err(e) => {
                            warn!("failed to read from l2cap channel: {:?}", e);
                            break;
                        }
                        Ok(len) => {
                            let buf = jarr.to_vec(env)?;
                            if let Err(e) = read_sender.write_all(&buf[..len as _]) {
                                warn!("failed to enqueue received l2cap packet: {:?}", e);
                                break;
                            }
                        }
                    }
                }
                Ok(())
            });

            debug!("l2cap read thread exiting!");
        });

        thread::spawn(move || {
            debug!("l2cap write thread running!");
            let mut write_receiver = BlockOn::new(write_receiver);
            let _ = jni_with_env(|env| {
                let mut buf = vec![0; PIPE_CAPACITY];
                loop {
                    match write_receiver.read(&mut buf) {
                        Err(e) => {
                            warn!("failed to dequeue l2cap packet to send: {:?}", e);
                            break;
                        }
                        Ok(0) => {
                            trace!("Stream ended");
                            break;
                        }
                        Ok(len) => {
                            let jarr = JByteArray::from_slice(env, &buf[..len])?;
                            if let Err(e) = output_stream.write(env, &jarr) {
                                warn!("failed to write to l2cap channel: {:?}", e);
                                break;
                            };
                        }
                    }
                }
                Ok(())
            });

            debug!("l2cap write thread exiting!");
        });

        Ok((
            L2capChannelReader {
                _closer: closer.clone(),
                stream: read_receiver,
            },
            L2capChannelWriter {
                _closer: closer,
                stream: write_sender,
            },
        ))
    })
}

/// Utility struct to close the channel on drop.
pub(super) struct L2capCloser {
    channel: Global<bindings::BluetoothSocket<'static>>,
}

impl L2capCloser {
    fn close(&self) {
        match jni_with_env(|env| Ok(self.channel.close(env)?)) {
            Ok(()) => debug!("l2cap channel closed"),
            Err(e) => warn!("failed to close channel: {:?}", e),
        }
    }
}

impl Drop for L2capCloser {
    fn drop(&mut self) {
        self.close()
    }
}

/// A Bluetooth LE L2CAP Connection-oriented Channel (CoC).
pub struct L2capChannel {
    pub(super) reader: L2capChannelReader,
    pub(super) writer: L2capChannelWriter,
}

impl L2capChannel {
    /// Split the channel into read and write halves.
    pub fn split(self) -> (L2capChannelReader, L2capChannelWriter) {
        (self.reader, self.writer)
    }
}

derive_async_read!(L2capChannel, reader);
derive_async_write!(L2capChannel, writer);

/// Reader half of a L2CAP Connection-oriented Channel (CoC).
pub struct L2capChannelReader {
    stream: piper::Reader,
    _closer: Arc<L2capCloser>,
}

derive_async_read!(L2capChannelReader, stream);

impl fmt::Debug for L2capChannelReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("L2capChannelReader")
    }
}

/// Writer half of a L2CAP Connection-oriented Channel (CoC).
pub struct L2capChannelWriter {
    stream: piper::Writer,
    _closer: Arc<L2capCloser>,
}

derive_async_write!(L2capChannelWriter, stream);

impl fmt::Debug for L2capChannelWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("L2capChannelWriter")
    }
}
