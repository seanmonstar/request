use super::{Body, Client, Response};

/// A request which can be executed with `Client::execute()`.
pub type Request = crate::core::request::Request<Body>;
/// A builder to construct the properties of a `Request`. The WASM client does not yet implement timeouts, and setting a timeout will not have any effect.
pub type RequestBuilder = crate::core::request::RequestBuilder<Client, Body>;

impl RequestBuilder {

    /// TODO
    pub fn multipart(mut self, multipart: super::multipart::Form) -> RequestBuilder {
        if let Ok(ref mut req) = self.request {
            *req.body_mut() = Some(Body::from_form(multipart))
        }
        self
    }

    /// Constructs the Request and sends it to the target URL, returning a
    /// future Response.
    ///
    /// # Errors
    ///
    /// This method fails if there was an error while sending request.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use reqwest::Error;
    /// #
    /// # async fn run() -> Result<(), Error> {
    /// let response = reqwest::Client::new()
    ///     .get("https://hyper.rs")
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send(self) -> crate::Result<Response> {
        let req = self.request?;
        self.client.execute_request(req).await
    }
}
