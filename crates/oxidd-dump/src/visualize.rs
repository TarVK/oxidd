use std::{io, result};

use oxidd_core::{function::Function, HasLevel, Manager};

use crate::dddmp::{export, AsciiDisplay};

/// Send the visualization to a given host api
///
/// 'dd_name' is the name that is sent to the visualization tool
///
/// `vars` are edges representing *all* variables in the decision diagram. The
/// order does not matter. `var_names` are the names of these variables
/// (optional). If given, there must be `vars.len()` names in the same order as
/// in `vars`.
///
/// `functions` are edges pointing to the root nodes of functions.
/// `function_names` are the corresponding names (optional). If given, there
/// must be `functions.len()` names in the same order as in `function_names`.
///
/// 'host' is the host domain to send the data to, which defaults to localhost:8080
///
pub fn visualize<'id, F: Function>(
    manager: &F::Manager<'id>,
    dd_name: &str,
    vars: &[&F],
    var_names: Option<&[&str]>,
    functions: &[&F],
    function_names: Option<&[&str]>,
    host: Option<&str>,
) -> Result<()>
where
    <F::Manager<'id> as Manager>::InnerNode: HasLevel,
    <F::Manager<'id> as Manager>::Terminal: AsciiDisplay,
{
    let mut out = FileOutput { data: Vec::new() };
    let export_result = export(
        &mut out,
        manager,
        true,
        dd_name,
        vars,
        var_names,
        functions,
        function_names,
        |_| false,
    );
    if let Err(e) = export_result {
        return Result::Err(Error::File(e));
    }

    let res = minreq::post(&format!(
        "{}/api/diagram?name={}&type=bdd",
        host.unwrap_or("http://127.0.0.1:8080"),
        dd_name
    ))
    .with_body(out.data.clone())
    .send();
    if let Err(e) = res {
        return Result::Err(Error::Http(e));
    }

    Ok(())
}

/// The result type of trying to visualize data
pub type Result<T> = result::Result<T, Error>;

/// Error data of attempting to visualize, which may fail when exporting or when sending a request
pub enum Error {
    /// File related error
    File(io::Error),
    /// Http related error
    Http(minreq::Error),
}

struct FileOutput {
    data: Vec<u8>,
}
impl io::Write for FileOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
