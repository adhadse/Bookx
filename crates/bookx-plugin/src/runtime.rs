//! The sandboxed WebAssembly runtime: one [`Plugin`] = one compiled module plus its limits.
//!
//! Every call builds a fresh `wasmi` store and instance, so nothing leaks from one call into the next
//! and a trapped or exhausted instance is simply dropped.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use wasmi::{
    Caller, Config, Engine, Extern, ExternType, FuncType, Linker, Memory, Module, Store,
    StoreLimits, StoreLimitsBuilder, TrapCode, ValType,
};

use crate::editor::{self, ActionResult, SharedSession};
use crate::manifest::{Manifest, PluginKind};

/// Largest reply a plugin may return (bytes). Anything bigger is an error.
pub const MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;
/// Largest response `http_get` hands to a plugin.
pub const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;
/// `http_get` calls allowed per plugin call.
pub const MAX_HTTP_CALLS: u32 = 16;
/// `log` lines kept per plugin call; later lines are dropped.
pub const MAX_LOG_LINES: u32 = 256;
/// Longest log line kept (bytes, truncated on a char boundary).
pub const MAX_LOG_BYTES: usize = 4096;
/// Fuel units that count as one millisecond of the manifest `timeout_ms`.
pub const FUEL_PER_MS: u64 = 50_000;

/// Error codes returned (negated) by `bookx_call`.
pub fn describe_error_code(code: i64) -> String {
    match code {
        -1 => "the plugin reported a failure (-1)".into(),
        -2 => "the plugin does not support this operation (-2)".into(),
        -3 => "the plugin rejected the input (-3)".into(),
        -4 => "the plugin ran out of memory (-4)".into(),
        c => format!("the plugin failed with code {c}"),
    }
}

/// Network access given to plugins that hold a `network:<host>` permission. The host decides how to
/// fetch; tests inject a mock so they never touch the network.
pub trait HostNetwork: Send + Sync {
    /// Fetches `url` (already checked against the plugin's permissions). Must not return more than
    /// `max_bytes`; the runtime rejects a longer body anyway.
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>>;
}

/// Default network: everything fails.
pub struct NoNetwork;

impl HostNetwork for NoNetwork {
    fn get(&self, _url: &str, _max_bytes: usize) -> Result<Vec<u8>> {
        bail!("networking is not available")
    }
}

/// Receives the plugin's `log` lines.
pub trait HostLog: Send + Sync {
    /// `level`: 0 trace, 1 debug, 2 info, 3 warn, 4 error (values above 4 are clamped to 4).
    fn log(&self, plugin_id: &str, level: u8, message: &str);
}

/// Default log: drops everything.
pub struct NullLog;

impl HostLog for NullLog {
    fn log(&self, _: &str, _: u8, _: &str) {}
}

/// Services the host lends to plugins.
#[derive(Clone)]
pub struct Host {
    pub network: Arc<dyn HostNetwork>,
    pub log: Arc<dyn HostLog>,
}

impl Default for Host {
    fn default() -> Self {
        Host {
            network: Arc::new(NoNetwork),
            log: Arc::new(NullLog),
        }
    }
}

impl Host {
    pub fn with_network(mut self, n: Arc<dyn HostNetwork>) -> Self {
        self.network = n;
        self
    }

    pub fn with_log(mut self, l: Arc<dyn HostLog>) -> Self {
        self.log = l;
        self
    }
}

/// What one call used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CallStats {
    pub fuel_used: u64,
    pub memory_bytes: usize,
    pub http_calls: u32,
}

struct State {
    plugin_id: String,
    hosts: Vec<String>,
    host: Host,
    limits: StoreLimits,
    deadline: Instant,
    http_calls: u32,
    log_lines: u32,
    editor: Option<SharedSession>,
}

/// A loaded plugin: compiled module + manifest + host services.
pub struct Plugin {
    manifest: Manifest,
    engine: Engine,
    module: Module,
    linker: Linker<State>,
    host: Host,
    max_reply: usize,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("id", &self.manifest.id)
            .field("kind", &self.manifest.kind)
            .finish()
    }
}

fn func_sig(params: &[ValType], results: &[ValType]) -> FuncType {
    FuncType::new(params.iter().copied(), results.iter().copied())
}

fn host_signature(name: &str) -> Option<FuncType> {
    use ValType::{I32, I64};
    match name {
        "log" => Some(func_sig(&[I32, I32, I32], &[])),
        "http_get" => Some(func_sig(&[I32, I32, I32], &[I32])),
        "now_ms" => Some(func_sig(&[], &[I64])),
        "editor_call" => Some(func_sig(&[I32, I32, I32, I32, I32], &[I32])),
        _ => None,
    }
}

fn required_exports() -> Vec<(&'static str, FuncType)> {
    use ValType::{I32, I64};
    vec![
        ("alloc", func_sig(&[I32], &[I32])),
        ("dealloc", func_sig(&[I32, I32], &[])),
        ("bookx_call", func_sig(&[I32, I32, I32, I32], &[I64])),
    ]
}

fn guest_memory<T>(caller: &mut Caller<'_, T>) -> Option<Memory> {
    caller.get_export("memory").and_then(Extern::into_memory)
}

fn read_guest<T>(caller: &mut Caller<'_, T>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    if ptr < 0 || len < 0 {
        return None;
    }
    let mem = guest_memory(caller)?;
    let mut buf = vec![0u8; len as usize];
    mem.read(&*caller, ptr as usize, &mut buf).ok()?;
    Some(buf)
}

/// Splits `scheme://host[:port]/…` into the lower-cased host. Only http and https are accepted.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.is_empty() || authority.starts_with('[') {
        return None;
    }
    let host = authority.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    if let Some((_, port)) = authority.split_once(':') {
        if port.is_empty()
            || !port.chars().all(|c| c.is_ascii_digit())
            || port.parse::<u16>().is_err()
        {
            return None;
        }
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn build_linker(engine: &Engine) -> Result<Linker<State>> {
    let mut linker: Linker<State> = Linker::new(engine);
    linker
        .func_wrap(
            "bookx_host",
            "log",
            |mut caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| {
                if caller.data().log_lines >= MAX_LOG_LINES {
                    return;
                }
                caller.data_mut().log_lines += 1;
                let len = len.clamp(0, MAX_LOG_BYTES as i32);
                let Some(bytes) = read_guest(&mut caller, ptr, len) else {
                    return;
                };
                let msg = String::from_utf8_lossy(&bytes).into_owned();
                let level = level.clamp(0, 4) as u8;
                let st = caller.data();
                st.host.log.log(&st.plugin_id, level, &msg);
            },
        )
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("bookx_host", "now_ms", |_: Caller<'_, State>| -> i64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap(
            "bookx_host",
            "http_get",
            |mut caller: Caller<'_, State>, url_ptr: i32, url_len: i32, out: i32| -> i32 {
                http_get(&mut caller, url_ptr, url_len, out)
            },
        )
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap(
            "bookx_host",
            "editor_call",
            |mut caller: Caller<'_, State>,
             op_ptr: i32,
             op_len: i32,
             in_ptr: i32,
             in_len: i32,
             out: i32|
             -> i32 { editor_call(&mut caller, op_ptr, op_len, in_ptr, in_len, out) },
        )
        .map_err(|e| anyhow!("{e}"))?;
    Ok(linker)
}

/// Longest input of one `editor_call` (bytes).
pub const MAX_EDITOR_INPUT: i32 = 32 * 1024 * 1024;

/// Copies `body` into guest memory through the guest's `alloc` and writes `ptr`, `len` at `out`.
fn give_to_guest(caller: &mut Caller<'_, State>, body: &[u8], out: i32) -> bool {
    let Some(alloc) = caller
        .get_export("alloc")
        .and_then(Extern::into_func)
        .and_then(|f| f.typed::<i32, i32>(&*caller).ok())
    else {
        return false;
    };
    let Ok(ptr) = alloc.call(&mut *caller, body.len() as i32) else {
        return false;
    };
    let Some(mem) = guest_memory(caller) else {
        return false;
    };
    if ptr < 0 || mem.write(&mut *caller, ptr as usize, body).is_err() {
        return false;
    }
    let mut cell = [0u8; 8];
    cell[..4].copy_from_slice(&(ptr as u32).to_le_bytes());
    cell[4..].copy_from_slice(&(body.len() as u32).to_le_bytes());
    out >= 0 && mem.write(&mut *caller, out as usize, &cell).is_ok()
}

/// `editor_call` result codes: see [`crate::editor`].
fn editor_call(
    caller: &mut Caller<'_, State>,
    op_ptr: i32,
    op_len: i32,
    in_ptr: i32,
    in_len: i32,
    out: i32,
) -> i32 {
    if !(0..=64).contains(&op_len) || !(0..=MAX_EDITOR_INPUT).contains(&in_len) {
        return editor::BAD;
    }
    let Some(session) = caller.data().editor.clone() else {
        return -6;
    };
    if Instant::now() >= caller.data().deadline {
        return -7;
    }
    let Some(op) = read_guest(caller, op_ptr, op_len).and_then(|b| String::from_utf8(b).ok())
    else {
        return editor::BAD;
    };
    let Some(input) = read_guest(caller, in_ptr, in_len) else {
        return editor::BAD;
    };
    let (code, reply) = session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .dispatch(&op, &input);
    let body = serde_json::to_vec(&reply).unwrap_or_default();
    if !give_to_guest(caller, &body, out) {
        return -5;
    }
    code
}

/// `http_get` result codes: 0 ok, -1 bad arguments / url, -2 host not permitted, -3 network failure,
/// -4 body too large, -5 guest allocation failed, -6 too many requests, -7 timed out.
fn http_get(caller: &mut Caller<'_, State>, url_ptr: i32, url_len: i32, out: i32) -> i32 {
    if !(0..=8192).contains(&url_len) {
        return -1;
    }
    let Some(url_bytes) = read_guest(caller, url_ptr, url_len) else {
        return -1;
    };
    let Ok(url) = String::from_utf8(url_bytes) else {
        return -1;
    };
    let Some(host) = url_host(&url) else {
        return -1;
    };
    if !caller.data().hosts.contains(&host) {
        return -2;
    }
    if caller.data().http_calls >= MAX_HTTP_CALLS {
        return -6;
    }
    if Instant::now() >= caller.data().deadline {
        return -7;
    }
    caller.data_mut().http_calls += 1;
    let net = caller.data().host.network.clone();
    let body = match net.get(&url, MAX_HTTP_BYTES) {
        Ok(b) => b,
        Err(_) => return -3,
    };
    if body.len() > MAX_HTTP_BYTES {
        return -4;
    }
    let Some(alloc) = caller
        .get_export("alloc")
        .and_then(Extern::into_func)
        .and_then(|f| f.typed::<i32, i32>(&*caller).ok())
    else {
        return -5;
    };
    let Ok(ptr) = alloc.call(&mut *caller, body.len() as i32) else {
        return -5;
    };
    let Some(mem) = guest_memory(caller) else {
        return -5;
    };
    if ptr < 0 || mem.write(&mut *caller, ptr as usize, &body).is_err() {
        return -5;
    }
    let mut cell = [0u8; 8];
    cell[..4].copy_from_slice(&(ptr as u32).to_le_bytes());
    cell[4..].copy_from_slice(&(body.len() as u32).to_le_bytes());
    if out < 0 || mem.write(&mut *caller, out as usize, &cell).is_err() {
        return -1;
    }
    0
}

impl Plugin {
    /// Compiles and validates the module (exports, imports) without running any guest code.
    pub fn from_bytes(manifest: Manifest, wasm: &[u8], host: Host) -> Result<Plugin> {
        let mut cfg = Config::default();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg);
        let module =
            Module::new(&engine, wasm).map_err(|e| anyhow!("invalid WebAssembly module: {e}"))?;
        for imp in module.imports() {
            let full = format!("{}::{}", imp.module(), imp.name());
            if imp.module() != "bookx_host" {
                bail!("import `{full}` is not allowed (only module `bookx_host`)");
            }
            let Some(expected) = host_signature(imp.name()) else {
                bail!("import `{full}` is not allowed (unknown host function)");
            };
            match imp.ty() {
                ExternType::Func(ft) if *ft == expected => {}
                ExternType::Func(_) => bail!("import `{full}` has the wrong signature"),
                _ => bail!("import `{full}` is not a function"),
            }
        }
        match module.get_export("memory") {
            Some(ExternType::Memory(_)) => {}
            Some(_) => bail!("export `memory` is not a linear memory"),
            None => bail!("the module must export its linear memory as `memory`"),
        }
        for (name, expected) in required_exports() {
            match module.get_export(name) {
                Some(ExternType::Func(ft)) if ft == expected => {}
                Some(ExternType::Func(_)) => bail!("export `{name}` has the wrong signature"),
                Some(_) => bail!("export `{name}` is not a function"),
                None => bail!("the module must export `{name}`"),
            }
        }
        if let Some(ty) = module.get_export("bookx_info") {
            match ty {
                ExternType::Func(ft) if ft == func_sig(&[], &[ValType::I64]) => {}
                _ => bail!("export `bookx_info` has the wrong signature"),
            }
        }
        for imp in module.imports() {
            if imp.name() == "editor_call" && manifest.kind != PluginKind::EditorAction {
                bail!("import `bookx_host::editor_call` is only for editor-action plugins");
            }
        }
        let linker = build_linker(&engine)?;
        Ok(Plugin {
            manifest,
            engine,
            module,
            linker,
            host,
            max_reply: MAX_REPLY_BYTES,
        })
    }

    /// Reads the `.wasm` file named by the manifest from `dir`.
    pub fn load(dir: &Path, manifest: Manifest, host: Host) -> Result<Plugin> {
        let path = manifest.wasm_path(dir);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(manifest, &bytes, host)
    }

    /// Lowers the reply size cap (tests, or hosts with less memory).
    pub fn with_max_reply(mut self, bytes: usize) -> Plugin {
        self.max_reply = bytes;
        self
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    fn new_store(&self, editor: Option<SharedSession>) -> Result<(Store<State>, Instant, u64)> {
        let lim = self.manifest.limits;
        let by_time = lim.timeout_ms.saturating_mul(FUEL_PER_MS);
        let fuel = lim.fuel.min(by_time).max(1);
        let timeout = Duration::from_millis(lim.timeout_ms);
        let deadline = Instant::now() + timeout;
        let state = State {
            plugin_id: self.manifest.id.clone(),
            hosts: self
                .manifest
                .allowed_hosts()
                .into_iter()
                .map(str::to_string)
                .collect(),
            host: self.host.clone(),
            limits: StoreLimitsBuilder::new()
                .memory_size(lim.memory_pages as usize * 65536)
                .instances(1)
                .memories(1)
                .tables(4)
                .table_elements(100_000)
                .trap_on_grow_failure(false)
                .build(),
            deadline,
            http_calls: 0,
            log_lines: 0,
            editor,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(fuel).map_err(|e| anyhow!("{e}"))?;
        Ok((store, deadline, fuel))
    }

    fn trap_message(&self, e: wasmi::Error, what: &str) -> anyhow::Error {
        match e.as_trap_code() {
            Some(TrapCode::OutOfFuel) => anyhow!(
                "plugin `{}` used up its time/instruction budget during {what}",
                self.manifest.id
            ),
            Some(code) => anyhow!(
                "plugin `{}` trapped during {what}: {code}",
                self.manifest.id
            ),
            None => anyhow!("plugin `{}` failed during {what}: {e}", self.manifest.id),
        }
    }

    /// Runs `bookx_call(op, input)` in a fresh instance and returns the raw reply bytes.
    pub fn call_raw(&self, op: &str, input: &[u8]) -> Result<(Vec<u8>, CallStats)> {
        self.call_raw_with(op, input, None)
    }

    /// Runs one editor action (`kind = "editor-action"`, ABI v2) against the book behind
    /// `session`. Permissions and limits are the ones the session was built with; the caller
    /// takes the book back out of the session afterwards, also when this fails.
    pub fn run_editor_action(&self, action: &str, session: &SharedSession) -> Result<ActionResult> {
        if self.manifest.kind != PluginKind::EditorAction {
            bail!(
                "plugin `{}` is not an editor-action plugin",
                self.manifest.id
            );
        }
        if !self.manifest.actions.iter().any(|a| a.id == action) {
            bail!(
                "plugin `{}` declares no action `{action}`",
                self.manifest.id
            );
        }
        let input = serde_json::to_vec(&serde_json::json!({ "action": action }))?;
        let (reply, _) = self.call_raw_with("run_action", &input, Some(session.clone()))?;
        let value = parse_envelope(&self.manifest.id, &reply)?;
        let guard = session.lock().unwrap_or_else(|e| e.into_inner());
        editor::action_result(value, &guard)
            .map_err(|e| anyhow!("plugin `{}`: {e}", self.manifest.id))
    }

    fn call_raw_with(
        &self,
        op: &str,
        input: &[u8],
        editor: Option<SharedSession>,
    ) -> Result<(Vec<u8>, CallStats)> {
        let id = &self.manifest.id;
        let (mut store, deadline, fuel0) = self.new_store(editor)?;
        let instance = self
            .linker
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|e| self.trap_message(e, "start-up"))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| anyhow!("plugin `{id}` has no memory"))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "alloc")
            .map_err(|e| anyhow!("{e}"))?;
        let bookx_call = instance
            .get_typed_func::<(i32, i32, i32, i32), i64>(&store, "bookx_call")
            .map_err(|e| anyhow!("{e}"))?;

        let put = |store: &mut Store<State>, data: &[u8]| -> Result<i32> {
            if data.len() > i32::MAX as usize {
                bail!("input is too large");
            }
            let ptr = alloc
                .call(&mut *store, data.len() as i32)
                .map_err(|e| self.trap_message(e, "alloc"))?;
            if ptr < 0 {
                bail!("plugin `{id}` returned an invalid buffer from alloc");
            }
            memory.write(&mut *store, ptr as usize, data).map_err(|_| {
                anyhow!("plugin `{id}` returned an out-of-bounds buffer from alloc")
            })?;
            Ok(ptr)
        };
        let op_ptr = put(&mut store, op.as_bytes())?;
        let in_ptr = put(&mut store, input)?;
        let ret = bookx_call
            .call(
                &mut store,
                (op_ptr, op.len() as i32, in_ptr, input.len() as i32),
            )
            .map_err(|e| self.trap_message(e, op))?;
        if Instant::now() > deadline {
            bail!(
                "plugin `{id}` exceeded its {} ms timeout",
                self.manifest.limits.timeout_ms
            );
        }
        if ret < 0 {
            bail!("plugin `{id}`: {}", describe_error_code(ret));
        }
        let ptr = ((ret as u64) >> 32) as usize;
        let len = (ret as u64 & 0xFFFF_FFFF) as usize;
        if len > self.max_reply {
            bail!(
                "plugin `{id}` returned a reply of {len} bytes (limit {})",
                self.max_reply
            );
        }
        let size = memory.data_size(&store);
        if ptr.checked_add(len).is_none_or(|end| end > size) {
            bail!("plugin `{id}` returned a reply outside its memory");
        }
        let mut reply = vec![0u8; len];
        memory
            .read(&store, ptr, &mut reply)
            .map_err(|_| anyhow!("plugin `{id}` returned a reply outside its memory"))?;
        let stats = CallStats {
            fuel_used: fuel0.saturating_sub(store.get_fuel().unwrap_or(0)),
            memory_bytes: size,
            http_calls: store.data().http_calls,
        };
        Ok((reply, stats))
    }

    /// Calls an operation with a JSON input and unwraps the reply envelope: `{"ok": value}` yields
    /// `value`, `{"err": "message"}` becomes an error. Anything else is a protocol violation.
    pub fn call_json(&self, op: &str, input: &Value) -> Result<Value> {
        let bytes = serde_json::to_vec(input)?;
        let (reply, _) = self.call_raw(op, &bytes)?;
        parse_envelope(&self.manifest.id, &reply)
    }

    /// Like [`Plugin::call_json`] but also returns the resource usage.
    pub fn call_json_stats(&self, op: &str, input: &Value) -> Result<(Value, CallStats)> {
        let bytes = serde_json::to_vec(input)?;
        let (reply, stats) = self.call_raw(op, &bytes)?;
        Ok((parse_envelope(&self.manifest.id, &reply)?, stats))
    }

    /// Calls the optional `bookx_info()` export; `None` when the plugin does not have one.
    pub fn info(&self) -> Result<Option<Value>> {
        if self.module.get_export("bookx_info").is_none() {
            return Ok(None);
        }
        let id = &self.manifest.id;
        let (mut store, _, _) = self.new_store(None)?;
        let instance = self
            .linker
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|e| self.trap_message(e, "start-up"))?;
        let f = instance
            .get_typed_func::<(), i64>(&store, "bookx_info")
            .map_err(|e| anyhow!("{e}"))?;
        let ret = f
            .call(&mut store, ())
            .map_err(|e| self.trap_message(e, "bookx_info"))?;
        if ret < 0 {
            bail!("plugin `{id}`: {}", describe_error_code(ret));
        }
        let ptr = ((ret as u64) >> 32) as usize;
        let len = (ret as u64 & 0xFFFF_FFFF) as usize;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| anyhow!("plugin `{id}` has no memory"))?;
        if len > 1024 * 1024 || ptr.saturating_add(len) > memory.data_size(&store) {
            bail!("plugin `{id}` returned an invalid info buffer");
        }
        let mut buf = vec![0u8; len];
        memory
            .read(&store, ptr, &mut buf)
            .map_err(|_| anyhow!("plugin `{id}` returned an invalid info buffer"))?;
        let v: Value = serde_json::from_slice(&buf)
            .map_err(|e| anyhow!("plugin `{id}` returned malformed info JSON: {e}"))?;
        Ok(Some(v))
    }
}

fn parse_envelope(id: &str, reply: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(reply)
        .map_err(|e| anyhow!("plugin `{id}` returned malformed JSON: {e}"))?;
    let Value::Object(mut map) = v else {
        bail!("plugin `{id}` reply is not a JSON object");
    };
    if let Some(e) = map.remove("err") {
        let msg = match e {
            Value::String(s) => s,
            other => other.to_string(),
        };
        bail!("plugin `{id}`: {msg}");
    }
    map.remove("ok")
        .ok_or_else(|| anyhow!("plugin `{id}` reply has neither `ok` nor `err`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_have_messages() {
        assert!(describe_error_code(-1).contains("failure"));
        assert!(describe_error_code(-2).contains("does not support"));
        assert!(describe_error_code(-3).contains("rejected"));
        assert!(describe_error_code(-4).contains("memory"));
        assert!(describe_error_code(-9).contains("code -9"));
    }

    #[test]
    fn envelope_parsing() {
        assert_eq!(
            parse_envelope("p", br#"{"ok":[1]}"#).unwrap(),
            serde_json::json!([1])
        );
        assert_eq!(parse_envelope("p", br#"{"ok":null}"#).unwrap(), Value::Null);
        let e = parse_envelope("p", br#"{"err":"boom"}"#)
            .unwrap_err()
            .to_string();
        assert_eq!(e, "plugin `p`: boom");
        // err wins over ok
        assert!(parse_envelope("p", br#"{"ok":1,"err":"x"}"#).is_err());
        assert!(parse_envelope("p", b"[]").is_err());
        assert!(parse_envelope("p", b"").is_err());
    }

    #[test]
    fn default_host_services_refuse_or_ignore() {
        let h = Host::default();
        assert!(h.network.get("https://x.org", 10).is_err());
        h.log.log("p", 2, "ignored");
        let h = h
            .with_network(Arc::new(NoNetwork))
            .with_log(Arc::new(NullLog));
        assert!(h.network.get("https://x.org", 10).is_err());
    }

    #[test]
    fn host_signatures_are_known() {
        assert!(host_signature("log").is_some());
        assert!(host_signature("http_get").is_some());
        assert!(host_signature("now_ms").is_some());
        assert!(host_signature("editor_call").is_some());
        assert!(host_signature("fd_write").is_none());
    }

    #[test]
    fn url_hosts() {
        assert_eq!(url_host("https://a.b/c").as_deref(), Some("a.b"));
        assert_eq!(url_host("gopher://a.b"), None);
    }
}
