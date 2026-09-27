//! Peripherals as host modules, after `blocks/Generics/PeripheralBlockEntity.java`.

use crate::events::{Event, EventManager, Label, Value};
use crate::host::Outgoing;

/// Events a module raised while it ran.
pub type Pending = Vec<(Label, Event)>;

/// Everything a module raised while it ran.
#[derive(Default)]
pub struct Raised {
    pub events: Pending,
    /// Messages for other machines, which only the world can route.
    pub posts: Vec<Outgoing>,
}

/// An access point broadcast, on its way to every point in range.
#[derive(Clone, Debug)]
pub struct Broadcast {
    /// Where the sending point stands.
    pub origin: [f64; 3],
    pub dimension: String,
    /// The sender's range decides who hears it.
    pub range: f64,
    pub args: Vec<Value>,
}

/// Something arriving at a module from outside its machine.
pub enum Incoming<'a> {
    Wireless(&'a Broadcast),
}

/// What a module may do while it runs.
pub struct Ctx<'a> {
    uuid: &'a str,
    type_name: &'a str,
    raised: Raised,
}

impl<'a> Ctx<'a> {
    fn new(uuid: &'a str, type_name: &'a str) -> Ctx<'a> {
        Ctx {
            uuid,
            type_name,
            raised: Raised::default(),
        }
    }

    /// Hands the world something to route.
    pub fn post(&mut self, message: Outgoing) {
        self.raised.posts.push(message);
    }

    pub fn uuid(&self) -> &str {
        self.uuid
    }

    /// Raises one of this peripheral's events, named after its type with the uuid and event name first.
    pub fn emit(&mut self, name: &str, args: impl IntoIterator<Item = Value>) {
        let mut payload = vec![Value::Str(self.uuid.into()), Value::Str(name.into())];
        payload.extend(args);
        self.raised
            .events
            .push((Label::Peripheral, Event::new(self.type_name, payload)));
    }

    /// Raises anything that is not shaped like a peripheral event.
    pub fn queue(&mut self, label: Label, event: Event) {
        self.raised.events.push((label, event));
    }
}

/// One attachable module.
pub trait Peripheral {
    /// `getTypeName`, namespaced.
    fn type_name(&self) -> &str;

    /// `getFunctionNames`.
    fn functions(&self) -> &[&'static str];

    /// `callFunction`, where `Err` reaches the guest as an `ExposedError`.
    fn call(&mut self, ctx: &mut Ctx, name: &str, args: &[Value]) -> Result<Vec<Value>, String>;

    /// A turn each tick, for a module that has to poll something.
    fn tick(&mut self, _ctx: &mut Ctx) {}

    /// Something arriving from outside the machine, ignored by default.
    fn receive(&mut self, _ctx: &mut Ctx, _incoming: &Incoming) {}

    /// Functions whose varargs are `@Primative`.
    fn primitive_only(&self) -> &[&'static str] {
        &[]
    }

    /// `isCompatibility`, true only for a peripheral bridged from another mod.
    fn is_compatibility(&self) -> bool {
        false
    }

    /// The host's end of this module's byte channel, if it has one.
    fn host_channel(&self) -> Option<crate::port::PortHandle> {
        None
    }
}

struct Attached {
    uuid: String,
    tag: String,
    module: Box<dyn Peripheral>,
}

/// Every module attached to one machine.
#[derive(Default)]
pub struct Peripherals {
    attached: Vec<Attached>,
}

impl Peripherals {
    pub fn new() -> Peripherals {
        Peripherals::default()
    }

    /// Attaches a module and returns its uuid, without raising `peripheralAttached`.
    pub fn attach(&mut self, machine: usize, module: Box<dyn Peripheral>) -> String {
        let uuid = uuid_for(machine, self.attached.len());
        self.attached.push(Attached {
            uuid: uuid.clone(),
            tag: String::new(),
            module,
        });
        uuid
    }

    /// The `peripheralAttached` a hot-plugged module raises.
    pub fn attached_event(&self, uuid: &str) -> Option<(Label, Event)> {
        let found = self.find(uuid)?;
        Some((
            Label::System,
            Event::new(
                "peripheralAttached",
                vec![
                    Value::Str(found.module.type_name().into()),
                    Value::Str(found.uuid.clone()),
                ],
            ),
        ))
    }

    /// Detaches a module, returning the `peripheralDetached` it raises.
    pub fn detach(&mut self, uuid: &str) -> Option<(Label, Event)> {
        let at = self.attached.iter().position(|a| a.uuid == uuid)?;
        let gone = self.attached.remove(at);
        Some((
            Label::System,
            Event::new(
                "peripheralDetached",
                vec![
                    Value::Str(gone.module.type_name().into()),
                    Value::Str(gone.uuid),
                ],
            ),
        ))
    }

    fn find(&self, uuid: &str) -> Option<&Attached> {
        self.attached.iter().find(|a| a.uuid == uuid)
    }

    pub fn ids(&self) -> Vec<String> {
        self.attached.iter().map(|a| a.uuid.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.attached.is_empty()
    }

    pub fn type_of(&self, uuid: &str) -> Option<&str> {
        Some(self.find(uuid)?.module.type_name())
    }

    pub fn tag_of(&self, uuid: &str) -> Option<&str> {
        Some(self.find(uuid)?.tag.as_str())
    }

    pub fn is_compatibility(&self, uuid: &str) -> Option<bool> {
        Some(self.find(uuid)?.module.is_compatibility())
    }

    pub fn functions(&self, uuid: &str) -> Option<Vec<String>> {
        Some(
            self.find(uuid)?
                .module
                .functions()
                .iter()
                .map(|f| f.to_string())
                .collect(),
        )
    }

    /// True when that function refuses anything but primitives.
    pub fn primitive_only(&self, uuid: &str, name: &str) -> bool {
        match self.find(uuid) {
            Some(found) => found.module.primitive_only().contains(&name),
            None => false,
        }
    }

    pub fn set_tag(&mut self, uuid: &str, tag: &str) -> bool {
        match self.attached.iter_mut().find(|a| a.uuid == uuid) {
            Some(found) => {
                found.tag = tag.to_string();
                true
            }
            None => false,
        }
    }

    pub fn query_tag(&self, tag: &str) -> Vec<String> {
        self.attached
            .iter()
            .filter(|a| a.tag == tag)
            .map(|a| a.uuid.clone())
            .collect()
    }

    /// Every byte channel on this machine, by the uuid its module was given.
    pub fn channels(&self) -> Vec<(String, crate::port::PortHandle)> {
        self.attached
            .iter()
            .filter_map(|a| Some((a.uuid.clone(), a.module.host_channel()?)))
            .collect()
    }

    /// The channel of the module carrying `uuid`.
    pub fn channel_of(&self, uuid: &str) -> Option<crate::port::PortHandle> {
        self.find(uuid)?.module.host_channel()
    }

    pub fn query_type(&self, type_name: &str) -> Vec<String> {
        self.attached
            .iter()
            .filter(|a| a.module.type_name() == type_name)
            .map(|a| a.uuid.clone())
            .collect()
    }

    /// `None` when nothing carries that uuid; the call's own failure is the inner `Err`.
    pub fn call(
        &mut self,
        uuid: &str,
        name: &str,
        args: &[Value],
    ) -> Option<(Result<Vec<Value>, String>, Raised)> {
        let found = self.attached.iter_mut().find(|a| a.uuid == uuid)?;
        if !found.module.functions().contains(&name) {
            return None;
        }
        let type_name = found.module.type_name().to_string();
        let mut ctx = Ctx::new(&found.uuid, &type_name);
        let result = found.module.call(&mut ctx, name, args);
        Some((result, ctx.raised))
    }

    /// Gives every module its turn, collecting what they raised.
    pub fn tick(&mut self) -> Raised {
        self.each(|module, ctx| module.tick(ctx))
    }

    /// Offers something from outside the machine to every module.
    pub fn receive(&mut self, incoming: &Incoming) -> Raised {
        self.each(|module, ctx| module.receive(ctx, incoming))
    }

    fn each(&mut self, mut visit: impl FnMut(&mut Box<dyn Peripheral>, &mut Ctx)) -> Raised {
        let mut raised = Raised::default();
        for attached in &mut self.attached {
            let type_name = attached.module.type_name().to_string();
            let mut ctx = Ctx::new(&attached.uuid, &type_name);
            visit(&mut attached.module, &mut ctx);
            raised.events.append(&mut ctx.raised.events);
            raised.posts.append(&mut ctx.raised.posts);
        }
        raised
    }
}

/// Queues `pending` on `events`, reporting whether anything was raised.
pub fn drain(events: &mut EventManager, pending: Pending) -> bool {
    let raised = !pending.is_empty();
    for (label, event) in pending {
        events.queue(label, event);
    }
    raised
}

/// A uuid that depends only on the machine and the slot.
fn uuid_for(machine: usize, index: usize) -> String {
    format!(
        "{:08x}-0000-4000-8000-{:012x}",
        machine as u32, index as u64
    )
}

/// Builds a module by type name.
pub fn build(type_name: &str, options: &toml::Table) -> Result<Box<dyn Peripheral>, String> {
    match type_name {
        Echo::TYPE => {
            reject_unknown(options, &[])?;
            Ok(Box::new(Echo::default()))
        }
        AccessPoint::TYPE => Ok(Box::new(AccessPoint::configure(options)?)),
        crate::port::Port::TYPE => {
            reject_unknown(options, &[])?;
            Ok(Box::new(crate::port::Port::new()))
        }
        _ => Err(format!(
            "unknown peripheral {type_name:?}; this build has {}",
            TYPES.join(", ")
        )),
    }
}

/// Every type `build` accepts.
pub const TYPES: [&str; 3] = [Echo::TYPE, AccessPoint::TYPE, crate::port::Port::TYPE];

/// Rejects an option the module does not know.
fn reject_unknown(options: &toml::Table, known: &[&str]) -> Result<(), String> {
    for key in options.keys() {
        if !known.contains(&key.as_str()) {
            return Err(match known.is_empty() {
                true => format!("option {key:?} is not one this peripheral takes"),
                false => format!("option {key:?} is not one of {}", known.join(", ")),
            });
        }
    }
    Ok(())
}

fn number(options: &toml::Table, key: &str) -> Result<Option<f64>, String> {
    match options.get(key) {
        None => Ok(None),
        Some(toml::Value::Integer(n)) => Ok(Some(*n as f64)),
        Some(toml::Value::Float(n)) => Ok(Some(*n)),
        Some(_) => Err(format!("option {key:?} must be a number")),
    }
}

fn vec3(options: &toml::Table, key: &str) -> Result<Option<[f64; 3]>, String> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };
    let toml::Value::Array(items) = value else {
        return Err(format!("option {key:?} must be three numbers"));
    };
    if items.len() != 3 {
        return Err(format!("option {key:?} must be three numbers"));
    }
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = match item {
            toml::Value::Integer(n) => *n as f64,
            toml::Value::Float(n) => *n,
            _ => return Err(format!("option {key:?} must be three numbers")),
        };
    }
    Ok(Some(out))
}

/// A module that does nothing, for the tests.
#[derive(Default)]
pub struct Echo {
    calls: i64,
}

impl Echo {
    pub const TYPE: &'static str = "neetemu:echo";
}

impl Peripheral for Echo {
    fn type_name(&self) -> &str {
        Echo::TYPE
    }

    fn functions(&self) -> &[&'static str] {
        &["echo", "calls", "ping", "fail"]
    }

    fn call(&mut self, ctx: &mut Ctx, name: &str, args: &[Value]) -> Result<Vec<Value>, String> {
        self.calls += 1;
        match name {
            "echo" => Ok(args.to_vec()),
            "calls" => Ok(vec![Value::Int(self.calls)]),
            "ping" => {
                ctx.emit("pong", args.to_vec());
                Ok(vec![])
            }
            "fail" => Err("echo asked to fail".into()),
            _ => Err(format!("no such function {name}")),
        }
    }
}

/// `blocks/AccessPoint/AccessPointBlockEntity.java`, placed at its own coordinates.
pub struct AccessPoint {
    pos: [f64; 3],
    dimension: String,
    range: i64,
}

impl AccessPoint {
    pub const TYPE: &'static str = "neetcomputers:access_point";
    /// `@Range(max = 500)`, whose `min` defaults to zero.
    pub const MAX_RANGE: i64 = 500;

    fn configure(options: &toml::Table) -> Result<AccessPoint, String> {
        reject_unknown(options, &["pos", "range", "dimension"])?;
        let range = number(options, "range")?.unwrap_or(Self::MAX_RANGE as f64) as i64;
        if !(0..=Self::MAX_RANGE).contains(&range) {
            return Err(format!("option \"range\" must be 0 to {}", Self::MAX_RANGE));
        }
        Ok(AccessPoint {
            pos: vec3(options, "pos")?.unwrap_or([0.0; 3]),
            dimension: match options.get("dimension") {
                Some(toml::Value::String(name)) => name.clone(),
                None => "overworld".to_string(),
                Some(_) => return Err("option \"dimension\" must be a name".into()),
            },
            range,
        })
    }
}

fn distance(from: [f64; 3], to: [f64; 3]) -> f64 {
    let squared: f64 = (0..3).map(|i| (from[i] - to[i]).powi(2)).sum();
    squared.sqrt()
}

impl Peripheral for AccessPoint {
    fn type_name(&self) -> &str {
        AccessPoint::TYPE
    }

    fn functions(&self) -> &[&'static str] {
        &["broadcast", "getRange", "setRange"]
    }

    fn primitive_only(&self) -> &[&'static str] {
        &["broadcast"]
    }

    fn call(&mut self, ctx: &mut Ctx, name: &str, args: &[Value]) -> Result<Vec<Value>, String> {
        match name {
            "broadcast" => {
                ctx.post(Outgoing::Wireless(Broadcast {
                    origin: self.pos,
                    dimension: self.dimension.clone(),
                    range: self.range as f64,
                    args: args.to_vec(),
                }));
                Ok(Vec::new())
            }
            "getRange" => Ok(vec![Value::Int(self.range)]),
            "setRange" => {
                let range = match args.first() {
                    Some(Value::Int(n)) => *n,
                    Some(Value::Num(n)) => *n as i64,
                    _ => return Err("#1 range: number expected".into()),
                };
                if !(0..=AccessPoint::MAX_RANGE).contains(&range) {
                    return Err(format!(
                        "#1 Number {range} not in range of 0-{}",
                        AccessPoint::MAX_RANGE
                    ));
                }
                self.range = range;
                Ok(Vec::new())
            }
            _ => Err(format!("Cant Find Function '{name}'")),
        }
    }

    /// `receive` puts the distance ahead of the payload; a point never hears itself.
    fn receive(&mut self, ctx: &mut Ctx, incoming: &Incoming) {
        let Incoming::Wireless(broadcast) = incoming;
        if broadcast.dimension != self.dimension || broadcast.origin == self.pos {
            return;
        }
        let away = distance(broadcast.origin, self.pos);
        if away > broadcast.range {
            return;
        }
        let mut args = vec![Value::Num(away)];
        args.extend(broadcast.args.iter().cloned());
        ctx.emit("received", args);
    }
}
