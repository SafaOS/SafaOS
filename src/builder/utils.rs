use std::io;

use curl::easy::{List, WriteError};

/// make a Get request to a URL and returns the results as a String
pub fn https_get(url: &str, headers: &[&str]) -> std::io::Result<String> {
    let mut response = Vec::new();
    https_get_write(url, headers, |data| {
        response.extend_from_slice(data);
        Ok(data.len())
    })?;

    Ok(String::from_utf8(response).unwrap())
}

/// make a Get request to a URL, writes the result chunk by chunk by calling the write_fn
pub fn https_get_write(
    url: &str,
    headers: &[&str],
    write_fn: impl FnMut(&[u8]) -> Result<usize, WriteError>,
) -> io::Result<()> {
    let mut headers_list = List::new();
    for header in headers {
        headers_list.append(header).unwrap();
    }

    let mut handle = curl::easy::Easy::new();

    handle.useragent(&format!("curl/{}", curl::Version::get().version()))?;
    handle.get(true)?;
    handle.follow_location(true)?;
    handle.url(url)?;
    handle.http_headers(headers_list)?;
    // sub-scope because handle has to be dropped before the response is used
    {
        let mut handle = handle.transfer();
        handle.write_function(write_fn)?;

        handle.perform()?;
    }

    Ok(())
}
