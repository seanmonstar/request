pub(crate) use self::imp::Decoder;

mod imp {
    use std::fmt;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[cfg(feature = "gzip")]
    use async_compression::stream::GzipDecoder;

    #[cfg(feature = "brotli")]
    use async_compression::stream::BrotliDecoder;

    use bytes::Bytes;
    use futures_core::Stream;
    use futures_util::stream::Peekable;
    use http::HeaderMap;

    use super::super::Body;
    use crate::error;

    /// A response decompressor over a non-blocking stream of chunks.
    ///
    /// The inner decoder may be constructed asynchronously.
    pub(crate) struct Decoder {
        inner: Inner,
    }

    enum DecoderType {
        #[cfg(feature = "gzip")]
        Gzip,
        #[cfg(feature = "brotli")]
        Brotli,
    }

    enum Inner {
        /// A `PlainText` decoder just returns the response content as is.
        PlainText(super::super::body::ImplStream),

        /// A `Gzip` decoder will uncompress the gzipped response content before returning it.
        #[cfg(feature = "gzip")]
        Gzip(GzipDecoder<Peekable<IoStream>>),

        /// A `Brotli` decoder will uncompress the brotlied response content before returning it.
        #[cfg(feature = "brotli")]
        Brotli(BrotliDecoder<Peekable<IoStream>>),

        /// A decoder that doesn't have a value yet.
        /// If gzip and brotli are not enabled, dead code will be triggered
        #[allow(dead_code)]
        Pending(Pending),
    }

    /// A future attempt to poll the response body for EOF so we know whether to use gzip or not.
    struct Pending(Peekable<IoStream>, DecoderType);

    struct IoStream(super::super::body::ImplStream);

    impl fmt::Debug for Decoder {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.debug_struct("Decoder").finish()
        }
    }

    impl Decoder {
        #[cfg(feature = "blocking")]
        pub(crate) fn empty() -> Decoder {
            Decoder {
                inner: Inner::PlainText(Body::empty().into_stream()),
            }
        }

        /// A plain text decoder.
        ///
        /// This decoder will emit the underlying chunks as-is.
        fn plain_text(body: Body) -> Decoder {
            Decoder {
                inner: Inner::PlainText(body.into_stream()),
            }
        }

        /// A gzip decoder.
        ///
        /// This decoder will buffer and decompress chunks that are gzipped.
        #[cfg(feature = "gzip")]
        fn gzip(body: Body) -> Decoder {
            use futures_util::StreamExt;

            Decoder {
                inner: Inner::Pending(Pending(
                    IoStream(body.into_stream()).peekable(),
                    DecoderType::Gzip,
                )),
            }
        }

        /// A brotli decoder.
        ///
        /// This decoder will buffer and decompress chunks that are brotlied.
        #[cfg(feature = "brotli")]
        fn brotli(body: Body) -> Decoder {
            use futures_util::StreamExt;

            Decoder {
                inner: Inner::Pending(Pending(
                    IoStream(body.into_stream()).peekable(),
                    DecoderType::Brotli,
                )),
            }
        }

        #[cfg(feature = "gzip")]
        fn detect_gzip(headers: &mut HeaderMap) -> bool {
            use http::header::{CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
            use log::warn;

            let content_encoding_gzip: bool;
            let mut is_gzip = {
                content_encoding_gzip = headers
                    .get_all(CONTENT_ENCODING)
                    .iter()
                    .any(|enc| enc == "gzip");
                content_encoding_gzip
                    || headers
                        .get_all(TRANSFER_ENCODING)
                        .iter()
                        .any(|enc| enc == "gzip")
            };
            if is_gzip {
                if let Some(content_length) = headers.get(CONTENT_LENGTH) {
                    if content_length == "0" {
                        warn!("gzip response with content-length of 0");
                        is_gzip = false;
                    }
                }
            }
            if is_gzip {
                headers.remove(CONTENT_ENCODING);
                headers.remove(CONTENT_LENGTH);
            }
            is_gzip
        }

        #[cfg(feature = "brotli")]
        fn detect_brotli(headers: &mut HeaderMap) -> bool {
            use http::header::{CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
            use log::warn;

            let content_encoding_gzip: bool;
            let mut is_brotli = {
                content_encoding_gzip = headers
                    .get_all(CONTENT_ENCODING)
                    .iter()
                    .any(|enc| enc == "br");
                content_encoding_gzip
                    || headers
                        .get_all(TRANSFER_ENCODING)
                        .iter()
                        .any(|enc| enc == "br")
            };
            if is_brotli {
                if let Some(content_length) = headers.get(CONTENT_LENGTH) {
                    if content_length == "0" {
                        warn!("brotli response with content-length of 0");
                        is_brotli = false;
                    }
                }
            }
            if is_brotli {
                headers.remove(CONTENT_ENCODING);
                headers.remove(CONTENT_LENGTH);
            }
            is_brotli
        }

        /// Constructs a Decoder from a hyper request.
        ///
        /// A decoder is just a wrapper around the hyper request that knows
        /// how to decode the content body of the request.
        ///
        /// Uses the correct variant by inspecting the Content-Encoding header.
        pub(crate) fn detect(
            _headers: &mut HeaderMap,
            body: Body,
            check_gzip: bool,
            check_brotli: bool,
        ) -> Decoder {
            if !check_gzip && !check_brotli {
                return Decoder::plain_text(body);
            }

            #[cfg(feature = "gzip")]
            {
                if Decoder::detect_gzip(_headers) {
                    return Decoder::gzip(body);
                }
            }

            #[cfg(feature = "brotli")]
            {
                if Decoder::detect_brotli(_headers) {
                    return Decoder::brotli(body);
                }
            }

            Decoder::plain_text(body)
        }
    }

    impl Stream for Decoder {
        type Item = Result<Bytes, error::Error>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
            // Do a read or poll for a pending decoder value.
            let new_value = match self.inner {
                Inner::Pending(ref mut future) => match Pin::new(future).poll(cx) {
                    Poll::Ready(Ok(inner)) => inner,
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Some(Err(crate::error::decode_io(e))));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                Inner::PlainText(ref mut body) => return Pin::new(body).poll_next(cx),
                #[cfg(feature = "gzip")]
                Inner::Gzip(ref mut decoder) => {
                    return match futures_core::ready!(Pin::new(decoder).poll_next(cx)) {
                        Some(Ok(bytes)) => Poll::Ready(Some(Ok(bytes))),
                        Some(Err(err)) => Poll::Ready(Some(Err(crate::error::decode_io(err)))),
                        None => Poll::Ready(None),
                    };
                }
                #[cfg(feature = "brotli")]
                Inner::Brotli(ref mut decoder) => {
                    return match futures_core::ready!(Pin::new(decoder).poll_next(cx)) {
                        Some(Ok(bytes)) => Poll::Ready(Some(Ok(bytes))),
                        Some(Err(err)) => Poll::Ready(Some(Err(crate::error::decode_io(err)))),
                        None => Poll::Ready(None),
                    };
                }
            };

            self.inner = new_value;
            self.poll_next(cx)
        }
    }

    impl Future for Pending {
        type Output = Result<Inner, std::io::Error>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            use futures_util::StreamExt;

            match futures_core::ready!(Pin::new(&mut self.0).poll_peek(cx)) {
                Some(Ok(_)) => {
                    // fallthrough
                }
                Some(Err(_e)) => {
                    // error was just a ref, so we need to really poll to move it
                    return Poll::Ready(Err(futures_core::ready!(
                        Pin::new(&mut self.0).poll_next(cx)
                    )
                    .expect("just peeked Some")
                    .unwrap_err()));
                }
                None => return Poll::Ready(Ok(Inner::PlainText(Body::empty().into_stream()))),
            };

            let _body = std::mem::replace(
                &mut self.0,
                IoStream(Body::empty().into_stream()).peekable(),
            );

            match self.1 {
                #[cfg(feature = "brotli")]
                DecoderType::Brotli => Poll::Ready(Ok(Inner::Brotli(BrotliDecoder::new(_body)))),
                #[cfg(feature = "gzip")]
                DecoderType::Gzip => Poll::Ready(Ok(Inner::Gzip(GzipDecoder::new(_body)))),
            }
        }
    }

    impl Stream for IoStream {
        type Item = Result<Bytes, std::io::Error>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
            match futures_core::ready!(Pin::new(&mut self.0).poll_next(cx)) {
                Some(Ok(chunk)) => Poll::Ready(Some(Ok(chunk))),
                Some(Err(err)) => Poll::Ready(Some(Err(err.into_io()))),
                None => Poll::Ready(None),
            }
        }
    }
}
