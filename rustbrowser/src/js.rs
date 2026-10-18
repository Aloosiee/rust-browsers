//! JavaScript runtime built on Boa.
//!
//! Rust implements the core DOM operations, timers, networking and storage as native functions;
//! `prelude.js` builds the rest of the web platform on top of them in JavaScript. Native
//! functions reach the page through a thread-local [`State`], because Boa function pointers
//! cannot capture Rust closures.

use crate::css::media_matches;
use crate::dom::NodeType;
use crate::jsdom::{Dom, FRAGMENT_TAG};
use crate::layout::Rect;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::object::builtins::JsArray;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Source, js_string,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use url::Url;

const PRELUDE: &str = include_str!("prelude.js");

type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

struct Timer {
    id: u32,
    due: Instant,
    interval: Option<Duration>,
    func: JsObject,
}

/// Things scripts asked the browser to do since the last frame.
#[derive(Default)]
pub struct Requests {
    pub alerts: Vec<String>,
    pub navigate: Option<Url>,
    pub history: Option<i32>,
    pub url_update: Option<Url>,
    pub scroll_to: Option<f32>,
    pub focus: Option<Option<usize>>,
}

/// Runtime state reachable from native functions. `generation` distinguishes the current page from replaced ones.
struct State {
    dom: Rc<RefCell<Dom>>,
    wrappers: HashMap<usize, JsObject>,
    protos: HashMap<&'static str, JsObject>,
    timers: Vec<Timer>,
    next_timer: u32,
    url: Url,
    viewport: (f32, f32),
    scroll: f32,
    rects: HashMap<usize, Rect>,
    focused: Option<usize>,
    req: Requests,
    generation: u64,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static STORAGE: RefCell<HashMap<String, HashMap<String, String>>> = RefCell::new(HashMap::new());
    static GENERATION: RefCell<u64> = const { RefCell::new(0) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("JS runtime not initialized")))
}

fn dom() -> Rc<RefCell<Dom>> {
    with(|s| s.dom.clone())
}

fn err(msg: impl Into<String>) -> JsError {
    JsNativeError::typ().with_message(msg.into()).into()
}

fn jstr(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn num(n: f64) -> JsValue {
    JsValue::from(n)
}

fn arg_string(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    match args.get(i) {
        Some(v) if !v.is_undefined() => Ok(v.to_string(ctx)?.to_std_string_escaped()),
        _ => Ok(String::new()),
    }
}

fn node_id(v: &JsValue, ctx: &mut Context) -> Option<usize> {
    let obj = v.as_object()?;
    let id = obj.get(js_string!("__nid"), ctx).ok()?;
    if id.is_undefined() {
        return None;
    }
    id.to_number(ctx).ok().map(|n| n as usize)
}

fn this_id(this: &JsValue, ctx: &mut Context) -> JsResult<usize> {
    node_id(this, ctx).ok_or_else(|| err("Illegal invocation: not a DOM node"))
}

fn arg_node(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<usize> {
    args.get(i)
        .and_then(|v| node_id(v, ctx))
        .ok_or_else(|| err("Argument is not a DOM node"))
}

/// Returns the JavaScript object for a DOM node, creating it on first use so identity is stable.
pub fn wrap(id: usize, ctx: &mut Context) -> JsValue {
    if let Some(o) = with(|s| s.wrappers.get(&id).cloned()) {
        return o.into();
    }
    let proto = with(|s| {
        let d = s.dom.borrow();
        let key = match &d.nodes[id].data {
            NodeType::Document => "document",
            NodeType::Element(_) => "element",
            NodeType::Text(_) => "text",
        };
        s.protos[key].clone()
    });
    let obj = JsObject::with_object_proto(ctx.intrinsics());
    obj.set_prototype(Some(proto));
    let _ = obj.define_property_or_throw(
        js_string!("__nid"),
        PropertyDescriptor::builder()
            .value(num(id as f64))
            .writable(false)
            .enumerable(false)
            .configurable(false),
        ctx,
    );
    with(|s| s.wrappers.insert(id, obj.clone()));
    obj.into()
}

fn wrap_opt(id: Option<usize>, ctx: &mut Context) -> JsValue {
    id.map(|i| wrap(i, ctx)).unwrap_or(JsValue::null())
}

fn list(ids: Vec<usize>, ctx: &mut Context) -> JsValue {
    let vals: Vec<JsValue> = ids.into_iter().map(|i| wrap(i, ctx)).collect();
    JsArray::from_iter(vals, ctx).into()
}

fn func(name: &str, f: Native, ctx: &mut Context) -> JsObject {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f))
        .name(JsString::from(name))
        .build()
        .into()
}

fn method(obj: &JsObject, name: &str, f: Native, ctx: &mut Context) {
    let fo = func(name, f, ctx);
    let _ = obj.define_property_or_throw(
        JsString::from(name),
        PropertyDescriptor::builder()
            .value(fo)
            .writable(true)
            .enumerable(false)
            .configurable(true),
        ctx,
    );
}

fn accessor(obj: &JsObject, name: &str, get: Native, set: Option<Native>, ctx: &mut Context) {
    let g = func(name, get, ctx);
    let s = set.map(|f| func(name, f, ctx));
    let _ = obj.define_property_or_throw(
        JsString::from(name),
        PropertyDescriptor::builder()
            .get(g)
            .maybe_set(s)
            .enumerable(true)
            .configurable(true),
        ctx,
    );
}

fn n_node_type(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let d = d.borrow();
    Ok(num(match &d.nodes[id].data {
        NodeType::Document => 9.0,
        NodeType::Element(e) if e.tag == FRAGMENT_TAG => 11.0,
        NodeType::Element(_) => 1.0,
        NodeType::Text(_) => 3.0,
    }))
}

fn n_node_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let d = d.borrow();
    Ok(jstr(&match &d.nodes[id].data {
        NodeType::Document => "#document".to_string(),
        NodeType::Element(e) if e.tag == FRAGMENT_TAG => "#document-fragment".to_string(),
        NodeType::Element(e) => e.tag.to_ascii_uppercase(),
        NodeType::Text(_) => "#text".to_string(),
    }))
}

fn rel_node(this: &JsValue, ctx: &mut Context, f: impl Fn(&Dom, usize) -> Option<usize>) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let r = {
        let d = dom();
        let d = d.borrow();
        f(&d, id)
    };
    Ok(wrap_opt(r, ctx))
}

fn sibling(d: &Dom, id: usize, offset: isize, elements_only: bool) -> Option<usize> {
    let p = d.nodes[id].parent?;
    let kids: Vec<usize> = d.nodes[p]
        .children
        .iter()
        .copied()
        .filter(|&c| !elements_only || c == id || d.is_element(c))
        .collect();
    let pos = kids.iter().position(|&c| c == id)? as isize + offset;
    if pos < 0 {
        return None;
    }
    kids.get(pos as usize).copied()
}

fn n_parent(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| d.nodes[id].parent)
}
fn n_parent_element(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| d.nodes[id].parent.filter(|&p| d.is_element(p)))
}
fn n_first_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| d.nodes[id].children.first().copied())
}
fn n_last_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| d.nodes[id].children.last().copied())
}
fn n_next(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| sibling(d, id, 1, false))
}
fn n_prev(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| sibling(d, id, -1, false))
}
fn e_next(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| sibling(d, id, 1, true))
}
fn e_prev(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| sibling(d, id, -1, true))
}
fn e_first(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| {
        d.nodes[id].children.iter().copied().find(|&c| d.is_element(c))
    })
}
fn e_last(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    rel_node(this, ctx, |d, id| {
        d.nodes[id].children.iter().rev().copied().find(|&c| d.is_element(c))
    })
}

fn n_child_nodes(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let kids = dom().borrow().nodes[id].children.clone();
    Ok(list(kids, ctx))
}

fn e_children(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let kids: Vec<usize> = {
        let d = dom();
        let d = d.borrow();
        d.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&c| d.is_element(c))
            .collect()
    };
    Ok(list(kids, ctx))
}

fn n_text_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let d = d.borrow();
    if matches!(d.nodes[id].data, NodeType::Document) {
        return Ok(JsValue::null());
    }
    Ok(jstr(&d.text_content(id)))
}

fn n_text_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let s = arg_string(args, 0, ctx)?;
    dom().borrow_mut().set_text_content(id, &s);
    Ok(JsValue::undefined())
}

fn n_value_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let d = d.borrow();
    Ok(match &d.nodes[id].data {
        NodeType::Text(t) => jstr(t),
        _ => JsValue::null(),
    })
}

fn n_connected(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let d = d.borrow();
    Ok(JsValue::from(id == 0 || d.ancestors(id).last() == Some(&0)))
}

fn dom_result(r: Result<(), String>) -> JsResult<()> {
    r.map_err(|e| JsNativeError::error().with_message(e).into())
}

fn n_append_child(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let child = arg_node(args, 0, ctx)?;
    dom_result(dom().borrow_mut().append(id, child))?;
    Ok(args[0].clone())
}

fn n_insert_before(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let child = arg_node(args, 0, ctx)?;
    let reference = args.get(1).and_then(|v| node_id(v, ctx));
    dom_result(dom().borrow_mut().insert_before(id, child, reference))?;
    Ok(args[0].clone())
}

fn n_remove_child(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let child = arg_node(args, 0, ctx)?;
    let d = dom();
    if d.borrow().nodes[child].parent != Some(id) {
        return Err(JsNativeError::error()
            .with_message("NotFoundError: node is not a child")
            .into());
    }
    d.borrow_mut().detach(child);
    Ok(args[0].clone())
}

fn n_replace_child(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let new = arg_node(args, 0, ctx)?;
    let old = arg_node(args, 1, ctx)?;
    let d = dom();
    dom_result(d.borrow_mut().insert_before(id, new, Some(old)))?;
    d.borrow_mut().detach(old);
    Ok(args[1].clone())
}

fn n_remove(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    dom().borrow_mut().detach(id);
    Ok(JsValue::undefined())
}

fn n_clone(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let deep = args.first().map_or(false, |v| v.to_boolean());
    let c = dom().borrow_mut().clone_node(id, deep);
    Ok(wrap(c, ctx))
}

fn n_contains(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let Some(other) = args.first().and_then(|v| node_id(v, ctx)) else {
        return Ok(JsValue::from(false));
    };
    let d = dom();
    let d = d.borrow();
    Ok(JsValue::from(other == id || d.ancestors(other).contains(&id)))
}

fn n_has_children(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(JsValue::from(!dom().borrow().nodes[id].children.is_empty()))
}

fn n_query(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let sel = arg_string(args, 0, ctx)?;
    let r = dom().borrow().query(id, &sel, true);
    match r {
        Ok(v) => Ok(wrap_opt(v.first().copied(), ctx)),
        Err(_) => Ok(JsValue::null()),
    }
}

fn n_query_all(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let sel = arg_string(args, 0, ctx)?;
    let r = dom().borrow().query(id, &sel, false).unwrap_or_default();
    Ok(list(r, ctx))
}

fn n_by_tag(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let tag = arg_string(args, 0, ctx)?.to_ascii_lowercase();
    let r = dom()
        .borrow()
        .find(id, |e| (tag == "*" || e.tag == tag) && e.tag != FRAGMENT_TAG);
    Ok(list(r, ctx))
}

fn n_by_class(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let classes: Vec<String> = arg_string(args, 0, ctx)?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let r = dom()
        .borrow()
        .find(id, |e| !classes.is_empty() && classes.iter().all(|c| e.has_class(c)));
    Ok(list(r, ctx))
}

fn with_element<R>(id: usize, f: impl FnOnce(&crate::dom::ElementData) -> R) -> JsResult<R> {
    let d = dom();
    let d = d.borrow();
    d.element(id).map(f).ok_or_else(|| err("Not an element"))
}

fn set_attr(id: usize, name: &str, value: Option<String>) -> JsResult<()> {
    let d = dom();
    let mut d = d.borrow_mut();
    let e = d.element_mut(id).ok_or_else(|| err("Not an element"))?;
    match value {
        Some(v) => {
            e.attrs.insert(name.to_ascii_lowercase(), v);
        }
        None => {
            e.attrs.remove(&name.to_ascii_lowercase());
        }
    }
    Ok(())
}

fn e_tag(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(jstr(&with_element(id, |e| e.tag.to_ascii_uppercase())?))
}

fn e_local_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(jstr(&with_element(id, |e| e.tag.clone())?))
}

fn e_get_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let name = arg_string(args, 0, ctx)?.to_ascii_lowercase();
    Ok(with_element(id, |e| e.attr(&name).map(jstr))?.unwrap_or(JsValue::null()))
}

fn e_set_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let name = arg_string(args, 0, ctx)?;
    let value = arg_string(args, 1, ctx)?;
    set_attr(id, &name, Some(value))?;
    Ok(JsValue::undefined())
}

fn e_remove_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let name = arg_string(args, 0, ctx)?;
    set_attr(id, &name, None)?;
    Ok(JsValue::undefined())
}

fn e_has_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let name = arg_string(args, 0, ctx)?.to_ascii_lowercase();
    Ok(JsValue::from(with_element(id, |e| e.attrs.contains_key(&name))?))
}

fn e_attr_names(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let names = with_element(id, |e| {
        let mut v: Vec<String> = e.attrs.keys().cloned().collect();
        v.sort();
        v
    })?;
    let vals: Vec<JsValue> = names.iter().map(|n| jstr(n)).collect();
    Ok(JsArray::from_iter(vals, ctx).into())
}

fn e_id_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(jstr(&with_element(id, |e| e.attr("id").unwrap_or("").to_string())?))
}

fn e_id_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let v = arg_string(args, 0, ctx)?;
    set_attr(id, "id", Some(v))?;
    Ok(JsValue::undefined())
}

fn e_class_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(jstr(&with_element(id, |e| e.attr("class").unwrap_or("").to_string())?))
}

fn e_class_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let v = arg_string(args, 0, ctx)?;
    set_attr(id, "class", Some(v))?;
    Ok(JsValue::undefined())
}

fn e_inner_html_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let mut out = String::new();
    dom().borrow().serialize_children(id, &mut out);
    Ok(jstr(&out))
}

fn e_inner_html_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let html = arg_string(args, 0, ctx)?;
    dom().borrow_mut().set_inner_html(id, &html);
    Ok(JsValue::undefined())
}

fn e_outer_html_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let mut out = String::new();
    dom().borrow().serialize(id, &mut out);
    Ok(jstr(&out))
}

fn e_outer_html_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let html = arg_string(args, 0, ctx)?;
    let d = dom();
    let mut d = d.borrow_mut();
    let Some(parent) = d.nodes[id].parent else {
        return Ok(JsValue::undefined());
    };
    for n in d.parse_fragment(&html) {
        let _ = d.insert_before(parent, n, Some(id));
    }
    d.detach(id);
    Ok(JsValue::undefined())
}

fn e_insert_adjacent_html(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let pos = arg_string(args, 0, ctx)?.to_ascii_lowercase();
    let html = arg_string(args, 1, ctx)?;
    let d = dom();
    let mut d = d.borrow_mut();
    let nodes = d.parse_fragment(&html);
    let parent = d.nodes[id].parent;
    for n in nodes {
        let r = match pos.as_str() {
            "beforebegin" => parent.map(|p| d.insert_before(p, n, Some(id))),
            "afterend" => parent.map(|p| {
                let next = sibling(&d, id, 1, false);
                d.insert_before(p, n, next)
            }),
            "afterbegin" => {
                let first = d.nodes[id].children.first().copied();
                Some(d.insert_before(id, n, first))
            }
            _ => Some(d.append(id, n)),
        };
        if let Some(r) = r {
            dom_result(r)?;
        }
    }
    Ok(JsValue::undefined())
}

fn form_value(d: &Dom, id: usize) -> String {
    let Some(e) = d.element(id) else { return String::new() };
    match e.tag.as_str() {
        "textarea" => e
            .attr("value")
            .map(str::to_string)
            .unwrap_or_else(|| d.text_content(id)),
        "select" => {
            let opts = d.find(id, |o| o.tag == "option");
            let chosen = opts
                .iter()
                .copied()
                .find(|&o| d.element(o).map_or(false, |o| o.attrs.contains_key("selected")))
                .or(opts.first().copied());
            chosen
                .map(|o| {
                    d.element(o)
                        .and_then(|e| e.attr("value").map(str::to_string))
                        .unwrap_or_else(|| d.text_content(o).trim().to_string())
                })
                .unwrap_or_default()
        }
        "input" if matches!(e.attr("type"), Some("checkbox") | Some("radio")) => {
            e.attr("value").unwrap_or("on").to_string()
        }
        _ => e.attr("value").unwrap_or("").to_string(),
    }
}

fn e_value_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let d = dom();
    let v = form_value(&d.borrow(), id);
    Ok(jstr(&v))
}

fn e_value_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let v = arg_string(args, 0, ctx)?;
    let d = dom();
    let is_select = d.borrow().element(id).map_or(false, |e| e.tag == "select");
    if is_select {
        let opts = d.borrow().find(id, |o| o.tag == "option");
        for o in opts {
            let val = form_value(&d.borrow(), o);
            let text = d.borrow().text_content(o);
            let pick = val == v || text.trim() == v;
            set_attr(o, "selected", pick.then(String::new))?;
        }
    } else {
        set_attr(id, "value", Some(v))?;
    }
    Ok(JsValue::undefined())
}

fn e_checked_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    Ok(JsValue::from(with_element(id, |e| e.attrs.contains_key("checked"))?))
}

fn e_checked_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let on = args.first().map_or(false, |v| v.to_boolean());
    set_attr(id, "checked", on.then(String::new))?;
    Ok(JsValue::undefined())
}

fn e_matches(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let sel = arg_string(args, 0, ctx)?;
    let Some(sels) = crate::css::parse_selector_list(&sel) else {
        return Ok(JsValue::from(false));
    };
    Ok(JsValue::from(dom().borrow().element_matches(id, &sels)))
}

fn e_closest(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let sel = arg_string(args, 0, ctx)?;
    let Some(sels) = crate::css::parse_selector_list(&sel) else {
        return Ok(JsValue::null());
    };
    let found = {
        let d = dom();
        let d = d.borrow();
        std::iter::once(id)
            .chain(d.ancestors(id))
            .find(|&i| d.element_matches(i, &sels))
    };
    Ok(wrap_opt(found, ctx))
}

fn e_rect(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this, ctx)?;
    let (r, scroll) = with(|s| (s.rects.get(&id).copied().unwrap_or_default(), s.scroll));
    let o = JsObject::with_object_proto(ctx.intrinsics());
    let top = r.y - scroll;
    for (k, v) in [
        ("x", r.x),
        ("y", top),
        ("left", r.x),
        ("top", top),
        ("width", r.w),
        ("height", r.h),
        ("right", r.x + r.w),
        ("bottom", top + r.h),
    ] {
        o.create_data_property_or_throw(JsString::from(k), num(v as f64), ctx)?;
    }
    Ok(o.into())
}

fn d_by_id(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = arg_string(args, 0, ctx)?;
    let found = dom()
        .borrow()
        .find(0, |e| e.attr("id") == Some(target.as_str()))
        .first()
        .copied();
    Ok(wrap_opt(found, ctx))
}

fn d_create_element(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let tag = arg_string(args, 0, ctx)?;
    let id = dom().borrow_mut().create_element(&tag);
    Ok(wrap(id, ctx))
}

fn d_create_text(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0, ctx)?;
    let id = dom().borrow_mut().create_text(&text);
    Ok(wrap(id, ctx))
}

fn d_create_fragment(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = dom().borrow_mut().create_element(FRAGMENT_TAG);
    Ok(wrap(id, ctx))
}

fn d_create_comment(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = dom().borrow_mut().create_text("");
    Ok(wrap(id, ctx))
}

fn d_write(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut html = String::new();
    for i in 0..args.len() {
        html.push_str(&arg_string(args, i, ctx)?);
    }
    let d = dom();
    let mut d = d.borrow_mut();
    let body = d.first_tag("body").unwrap_or(0);
    for n in d.parse_fragment(&html) {
        let _ = d.append(body, n);
    }
    Ok(JsValue::undefined())
}

fn h_set_timer(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let f = args
        .first()
        .and_then(|v| v.as_object())
        .filter(|o| o.is_callable())
        .ok_or_else(|| err("Timer callback is not a function"))?;
    let ms = args.get(1).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0);
    let ms = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
    let repeat = args.get(2).map_or(false, |v| v.to_boolean());
    let d = Duration::from_micros((ms * 1000.0) as u64);
    let id = with(|s| {
        s.next_timer += 1;
        let id = s.next_timer;
        s.timers.push(Timer {
            id,
            due: Instant::now() + d,
            interval: repeat.then_some(d.max(Duration::from_millis(4))),
            func: f,
        });
        id
    });
    Ok(num(id as f64))
}

fn h_clear_timer(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = args.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u32;
    with(|s| s.timers.retain(|t| t.id != id));
    Ok(JsValue::undefined())
}

fn h_alert(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let msg = arg_string(args, 0, ctx)?;
    with(|s| s.req.alerts.push(msg));
    Ok(JsValue::undefined())
}

fn h_log(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let level = arg_string(args, 0, ctx)?;
    let msg = arg_string(args, 1, ctx)?;
    eprintln!("[console.{}] {}", level, msg);
    Ok(JsValue::undefined())
}

fn resolve_url(href: &str) -> Option<Url> {
    with(|s| s.url.join(href).ok())
}

fn h_navigate(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let href = arg_string(args, 0, ctx)?;
    if let Some(u) = resolve_url(&href) {
        with(|s| s.req.navigate = Some(u));
    }
    Ok(JsValue::undefined())
}

fn h_history(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let n = args.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as i32;
    with(|s| s.req.history = Some(n));
    Ok(JsValue::undefined())
}

fn h_push_state(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let href = arg_string(args, 0, ctx)?;
    if href.is_empty() {
        return Ok(JsValue::undefined());
    }
    if let Some(u) = resolve_url(&href) {
        let same_origin = with(|s| s.url.origin() == u.origin());
        if !same_origin {
            return Err(JsNativeError::error()
                .with_message("SecurityError: cross-origin pushState")
                .into());
        }
        with(|s| {
            s.url = u.clone();
            s.req.url_update = Some(u);
        });
    }
    Ok(JsValue::undefined())
}

fn url_object(u: &Url, ctx: &mut Context) -> JsResult<JsValue> {
    let o = JsObject::with_object_proto(ctx.intrinsics());
    let host = match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{}:{}", h, p),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    };
    let fields = [
        ("href", u.to_string()),
        ("protocol", format!("{}:", u.scheme())),
        ("host", host),
        ("hostname", u.host_str().unwrap_or("").to_string()),
        ("port", u.port().map(|p| p.to_string()).unwrap_or_default()),
        ("pathname", u.path().to_string()),
        ("search", u.query().map(|q| format!("?{}", q)).unwrap_or_default()),
        (
            "hash",
            u.fragment()
                .filter(|f| !f.is_empty())
                .map(|f| format!("#{}", f))
                .unwrap_or_default(),
        ),
        ("origin", u.origin().ascii_serialization()),
    ];
    for (k, v) in fields {
        o.create_data_property_or_throw(JsString::from(k), jstr(&v), ctx)?;
    }
    Ok(o.into())
}

fn h_location(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let u = with(|s| s.url.clone());
    url_object(&u, ctx)
}

fn h_parse_url(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let input = arg_string(args, 0, ctx)?;
    let base = match args.get(1) {
        Some(b) if !b.is_undefined() && !b.is_null() => Some(b.to_string(ctx)?.to_std_string_escaped()),
        _ => None,
    };
    let parsed = match base {
        Some(b) => Url::parse(&b).and_then(|b| b.join(&input)),
        None => Url::parse(&input),
    };
    match parsed {
        Ok(u) => url_object(&u, ctx),
        Err(_) => Err(err(format!("Invalid URL: {}", input))),
    }
}

fn h_resolve(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let href = arg_string(args, 0, ctx)?;
    Ok(jstr(&resolve_url(&href).map(|u| u.to_string()).unwrap_or(href)))
}

fn h_storage(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = arg_string(args, 0, ctx)?;
    let key = arg_string(args, 1, ctx)?;
    let value = arg_string(args, 2, ctx)?;
    let session = args.get(3).map_or(false, |v| v.to_boolean());
    let ns = with(|s| {
        format!(
            "{}{}",
            if session { "session:" } else { "local:" },
            s.url.origin().ascii_serialization()
        )
    });
    let out = STORAGE.with(|st| {
        let mut st = st.borrow_mut();
        let m = st.entry(ns).or_default();
        match op.as_str() {
            "get" => m.get(&key).map(|v| jstr(v)).unwrap_or(JsValue::null()),
            "set" => {
                m.insert(key, value);
                JsValue::undefined()
            }
            "remove" => {
                m.remove(&key);
                JsValue::undefined()
            }
            "clear" => {
                m.clear();
                JsValue::undefined()
            }
            "keys" => {
                let mut keys: Vec<String> = m.keys().cloned().collect();
                keys.sort();
                jstr(&keys.join("\u{1}"))
            }
            _ => JsValue::undefined(),
        }
    });
    Ok(out)
}

fn h_fetch_sync(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let href = arg_string(args, 0, ctx)?;
    let method = arg_string(args, 1, ctx)?.to_ascii_uppercase();
    let url = resolve_url(&href).ok_or_else(|| err("Invalid URL"))?;
    if method != "GET" && !method.is_empty() {
        return Err(err(format!("{} requests are not supported", method)));
    }
    let resp = crate::net::fetch(&url).map_err(|e| err(format!("Network error: {}", e)))?;
    let text = crate::net::decode(&resp.body, &resp.content_type);
    let o = JsObject::with_object_proto(ctx.intrinsics());
    o.create_data_property_or_throw(js_string!("status"), num(resp.status as f64), ctx)?;
    o.create_data_property_or_throw(js_string!("url"), jstr(resp.url.as_str()), ctx)?;
    o.create_data_property_or_throw(js_string!("contentType"), jstr(&resp.content_type), ctx)?;
    o.create_data_property_or_throw(js_string!("body"), jstr(&text), ctx)?;
    Ok(o.into())
}

fn h_viewport(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (w, h, s) = with(|s| (s.viewport.0, s.viewport.1, s.scroll));
    Ok(JsArray::from_iter([num(w as f64), num(h as f64), num(s as f64)], ctx).into())
}

fn h_scroll_to(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let y = args.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0);
    if y.is_finite() {
        with(|s| s.req.scroll_to = Some(y.max(0.0) as f32));
    }
    Ok(JsValue::undefined())
}

fn h_focus(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = args.first().and_then(|v| node_id(v, ctx));
    with(|s| {
        s.focused = id;
        s.req.focus = Some(id);
    });
    Ok(JsValue::undefined())
}

fn h_active(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = with(|s| s.focused).or_else(|| dom().borrow().first_tag("body"));
    Ok(wrap_opt(id, ctx))
}

fn h_media(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let q = arg_string(args, 0, ctx)?.to_ascii_lowercase();
    let w = with(|s| s.viewport.0);
    Ok(JsValue::from(media_matches(&q, w)))
}

/// Event properties supplied by the browser when it dispatches an event.
pub struct EventInit {
    pub kind: &'static str,
    pub bubbles: bool,
    pub client: Option<(f32, f32)>,
    pub key: Option<String>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl EventInit {
    pub fn simple(bubbles: bool) -> EventInit {
        EventInit {
            kind: "event",
            bubbles,
            client: None,
            key: None,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
}

/// A Boa context bound to one page's DOM.
pub struct JsRuntime {
    ctx: Context,
    generation: u64,
}

impl JsRuntime {
    /// Creates a runtime for a page and installs the DOM bindings and prelude.
    pub fn new(dom: Rc<RefCell<Dom>>, url: Url) -> JsRuntime {
        let generation = GENERATION.with(|g| {
            *g.borrow_mut() += 1;
            *g.borrow()
        });
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                dom,
                wrappers: HashMap::new(),
                protos: HashMap::new(),
                timers: Vec::new(),
                next_timer: 0,
                url,
                viewport: (1024.0, 768.0),
                scroll: 0.0,
                rects: HashMap::new(),
                focused: None,
                req: Requests::default(),
                generation,
            })
        });
        let mut ctx = Context::default();
        ctx.runtime_limits_mut().set_loop_iteration_limit(200_000_000);
        ctx.runtime_limits_mut().set_recursion_limit(2000);
        let mut rt = JsRuntime { ctx, generation };
        rt.install();
        rt
    }

    /// False once a newer page has replaced this runtime; calls on a stale runtime do nothing.
    fn active(&self) -> bool {
        STATE.with(|s| s.borrow().as_ref().map_or(false, |st| st.generation == self.generation))
    }

    fn install(&mut self) {
        let ctx = &mut self.ctx;
        let node = JsObject::with_object_proto(ctx.intrinsics());
        let accessors: &[(&str, Native, Option<Native>)] = &[
            ("nodeType", n_node_type, None),
            ("nodeName", n_node_name, None),
            ("parentNode", n_parent, None),
            ("parentElement", n_parent_element, None),
            ("childNodes", n_child_nodes, None),
            ("firstChild", n_first_child, None),
            ("lastChild", n_last_child, None),
            ("nextSibling", n_next, None),
            ("previousSibling", n_prev, None),
            ("textContent", n_text_get, Some(n_text_set)),
            ("nodeValue", n_value_get, Some(n_text_set)),
            ("data", n_value_get, Some(n_text_set)),
            ("isConnected", n_connected, None),
            ("children", e_children, None),
            ("firstElementChild", e_first, None),
            ("lastElementChild", e_last, None),
        ];
        for (name, g, s) in accessors {
            accessor(&node, name, *g, *s, ctx);
        }
        let methods: &[(&str, Native)] = &[
            ("appendChild", n_append_child),
            ("insertBefore", n_insert_before),
            ("removeChild", n_remove_child),
            ("replaceChild", n_replace_child),
            ("remove", n_remove),
            ("cloneNode", n_clone),
            ("contains", n_contains),
            ("hasChildNodes", n_has_children),
            ("querySelector", n_query),
            ("querySelectorAll", n_query_all),
            ("getElementsByTagName", n_by_tag),
            ("getElementsByClassName", n_by_class),
        ];
        for (name, f) in methods {
            method(&node, name, *f, ctx);
        }

        let element = JsObject::with_object_proto(ctx.intrinsics());
        element.set_prototype(Some(node.clone()));
        let accessors: &[(&str, Native, Option<Native>)] = &[
            ("tagName", e_tag, None),
            ("localName", e_local_name, None),
            ("id", e_id_get, Some(e_id_set)),
            ("className", e_class_get, Some(e_class_set)),
            ("innerHTML", e_inner_html_get, Some(e_inner_html_set)),
            ("outerHTML", e_outer_html_get, Some(e_outer_html_set)),
            ("nextElementSibling", e_next, None),
            ("previousElementSibling", e_prev, None),
            ("value", e_value_get, Some(e_value_set)),
            ("checked", e_checked_get, Some(e_checked_set)),
        ];
        for (name, g, s) in accessors {
            accessor(&element, name, *g, *s, ctx);
        }
        let methods: &[(&str, Native)] = &[
            ("getAttribute", e_get_attr),
            ("setAttribute", e_set_attr),
            ("removeAttribute", e_remove_attr),
            ("hasAttribute", e_has_attr),
            ("getAttributeNames", e_attr_names),
            ("matches", e_matches),
            ("closest", e_closest),
            ("getBoundingClientRect", e_rect),
            ("insertAdjacentHTML", e_insert_adjacent_html),
        ];
        for (name, f) in methods {
            method(&element, name, *f, ctx);
        }

        let text = JsObject::with_object_proto(ctx.intrinsics());
        text.set_prototype(Some(node.clone()));

        let document = JsObject::with_object_proto(ctx.intrinsics());
        document.set_prototype(Some(node.clone()));
        let methods: &[(&str, Native)] = &[
            ("getElementById", d_by_id),
            ("createElement", d_create_element),
            ("createTextNode", d_create_text),
            ("createDocumentFragment", d_create_fragment),
            ("createComment", d_create_comment),
            ("write", d_write),
        ];
        for (name, f) in methods {
            method(&document, name, *f, ctx);
        }

        with(|s| {
            s.protos.insert("node", node.clone());
            s.protos.insert("element", element.clone());
            s.protos.insert("text", text.clone());
            s.protos.insert("document", document.clone());
        });

        let host = JsObject::with_object_proto(ctx.intrinsics());
        let methods: &[(&str, Native)] = &[
            ("setTimer", h_set_timer),
            ("clearTimer", h_clear_timer),
            ("alert", h_alert),
            ("log", h_log),
            ("navigate", h_navigate),
            ("history", h_history),
            ("pushState", h_push_state),
            ("location", h_location),
            ("parseUrl", h_parse_url),
            ("resolve", h_resolve),
            ("storage", h_storage),
            ("fetchSync", h_fetch_sync),
            ("viewport", h_viewport),
            ("scrollTo", h_scroll_to),
            ("focus", h_focus),
            ("activeElement", h_active),
            ("mediaMatches", h_media),
        ];
        for (name, f) in methods {
            method(&host, name, *f, ctx);
        }
        let protos = JsObject::with_object_proto(ctx.intrinsics());
        for (k, v) in [
            ("node", &node),
            ("element", &element),
            ("text", &text),
            ("document", &document),
        ] {
            let _ = protos.create_data_property_or_throw(JsString::from(k), v.clone(), ctx);
        }
        let _ = host.create_data_property_or_throw(js_string!("protos"), protos, ctx);
        let doc = wrap(0, ctx);
        let global = ctx.global_object();
        let _ = global.create_data_property_or_throw(js_string!("__host"), host, ctx);
        let _ = global.create_data_property_or_throw(js_string!("document"), doc, ctx);
        self.exec(PRELUDE, "prelude");
    }

    pub fn exec(&mut self, code: &str, name: &str) {
        if !self.active() {
            return;
        }
        if let Err(e) = self.ctx.eval(Source::from_bytes(code)) {
            eprintln!("[js error] {}: {}", name, e);
        }
        self.run_jobs();
    }

    fn run_jobs(&mut self) {
        if let Err(e) = self.ctx.run_jobs() {
            eprintln!("[js error] promise job: {}", e);
        }
    }

    /// Runs classic scripts in document order, then fires `DOMContentLoaded` and `load`.
    pub fn run_page_scripts(&mut self) {
        let scripts: Vec<(Option<String>, String)> = {
            let d = dom();
            let d = d.borrow();
            d.find(0, |e| e.tag == "script")
                .into_iter()
                .filter(|&id| {
                    let e = d.element(id).unwrap();
                    let t = e.attr("type").unwrap_or("").trim().to_ascii_lowercase();
                    t.is_empty() || t.contains("javascript") || t.contains("ecmascript")
                })
                .map(|id| {
                    let e = d.element(id).unwrap();
                    (e.attr("src").map(str::to_string), d.text_content(id))
                })
                .collect()
        };
        for (src, inline) in scripts {
            match src {
                Some(src) => {
                    let Some(u) = resolve_url(&src) else { continue };
                    match crate::net::fetch(&u) {
                        Ok(r) if r.status < 400 => {
                            let code = crate::net::decode(&r.body, &r.content_type);
                            self.exec(&code, u.as_str());
                        }
                        Ok(r) => eprintln!("[js] {} returned HTTP {}", u, r.status),
                        Err(e) => eprintln!("[js] couldn't load {}: {}", u, e),
                    }
                }
                None => self.exec(&inline, "inline script"),
            }
        }
        self.exec("__setReadyState('interactive')", "lifecycle");
        self.dispatch(Some(0), "DOMContentLoaded", EventInit::simple(true));
        self.exec("__setReadyState('complete')", "lifecycle");
        self.dispatch(None, "load", EventInit::simple(false));
    }

    /// Dispatches a trusted event through the prelude. Returns true if a listener called `preventDefault`.
    pub fn dispatch(&mut self, target: Option<usize>, ty: &str, init: EventInit) -> bool {
        if !self.active() {
            return false;
        }
        let ctx = &mut self.ctx;
        let target_val = match target {
            Some(id) => wrap(id, ctx),
            None => JsValue::null(),
        };
        let o = JsObject::with_object_proto(ctx.intrinsics());
        let _ = o.create_data_property_or_throw(js_string!("kind"), jstr(init.kind), ctx);
        let _ = o.create_data_property_or_throw(js_string!("bubbles"), JsValue::from(init.bubbles), ctx);
        let _ = o.create_data_property_or_throw(js_string!("cancelable"), JsValue::from(true), ctx);
        let _ = o.create_data_property_or_throw(js_string!("ctrlKey"), JsValue::from(init.ctrl), ctx);
        let _ = o.create_data_property_or_throw(js_string!("shiftKey"), JsValue::from(init.shift), ctx);
        let _ = o.create_data_property_or_throw(js_string!("altKey"), JsValue::from(init.alt), ctx);
        if let Some((x, y)) = init.client {
            let _ = o.create_data_property_or_throw(js_string!("clientX"), num(x as f64), ctx);
            let _ = o.create_data_property_or_throw(js_string!("clientY"), num(y as f64), ctx);
        }
        if let Some(k) = &init.key {
            let _ = o.create_data_property_or_throw(js_string!("key"), jstr(k), ctx);
        }
        let global = ctx.global_object();
        let prevented = match global.get(js_string!("__hostDispatch"), ctx) {
            Ok(f) => match f.as_object() {
                Some(f) => match f.call(&JsValue::undefined(), &[target_val, jstr(ty), o.into()], ctx) {
                    Ok(v) => v.to_boolean(),
                    Err(e) => {
                        eprintln!("[js error] dispatching {}: {}", ty, e);
                        false
                    }
                },
                None => false,
            },
            Err(_) => false,
        };
        self.run_jobs();
        prevented
    }

    /// Runs timers that are due. Returns true if any ran.
    pub fn tick(&mut self) -> bool {
        if !self.active() {
            return false;
        }
        let now = Instant::now();
        let due: Vec<(u32, JsObject)> = with(|s| {
            let mut out = Vec::new();
            let mut keep = Vec::new();
            s.timers.sort_by_key(|t| t.due);
            for mut t in s.timers.drain(..) {
                if t.due <= now && out.len() < 64 {
                    out.push((t.id, t.func.clone()));
                    if let Some(iv) = t.interval {
                        t.due = now + iv;
                        keep.push(t);
                    }
                } else {
                    keep.push(t);
                }
            }
            s.timers = keep;
            out
        });
        if due.is_empty() {
            return false;
        }
        for (_, f) in due {
            if let Err(e) = f.call(&JsValue::undefined(), &[], &mut self.ctx) {
                eprintln!("[js error] timer: {}", e);
            }
            self.run_jobs();
        }
        true
    }

    pub fn set_layout_info(&self, viewport: (f32, f32), scroll: f32, rects: HashMap<usize, Rect>) {
        if !self.active() {
            return;
        }
        with(|s| {
            s.viewport = viewport;
            s.scroll = scroll;
            s.rects = rects;
        });
    }

    pub fn set_scroll(&self, scroll: f32) {
        if self.active() {
            with(|s| s.scroll = scroll);
        }
    }

    pub fn set_focus(&self, id: Option<usize>) {
        if self.active() {
            with(|s| s.focused = id);
        }
    }

    pub fn take_requests(&self) -> Requests {
        if !self.active() {
            return Requests::default();
        }
        with(|s| std::mem::take(&mut s.req))
    }
}

impl Drop for JsRuntime {
    /// Releases the page's JavaScript objects while the Boa context is still alive.
    /// Leaving them in the thread-local until thread exit would free them after Boa's
    /// own thread-local collector is gone.
    fn drop(&mut self) {
        if self.active() {
            let state = STATE.with(|s| s.borrow_mut().take());
            drop(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(html: &str) -> (Rc<RefCell<Dom>>, JsRuntime) {
        let dom = Rc::new(RefCell::new(Dom::from_tree(&crate::html::parse(html))));
        let rt = JsRuntime::new(dom.clone(), Url::parse("https://example.com/page").unwrap());
        (dom, rt)
    }

    fn text_of(dom: &Rc<RefCell<Dom>>, id_attr: &str) -> String {
        let d = dom.borrow();
        let id = d.find(0, |e| e.id() == Some(id_attr))[0];
        d.text_content(id)
    }

    #[test]
    fn scripts_mutate_the_dom() {
        let (dom, mut rt) = runtime("<ul id=list></ul><p id=out></p>");
        rt.exec(
            "const ul = document.getElementById('list');
             for (const t of ['a', 'b']) { const li = document.createElement('li'); li.textContent = t; ul.appendChild(li); }
             document.getElementById('out').textContent = ul.children.length + ':' + ul.innerHTML;",
            "test",
        );
        assert_eq!(text_of(&dom, "out"), "2:<li>a</li><li>b</li>");
    }

    #[test]
    fn events_bubble_and_can_be_cancelled() {
        let (dom, mut rt) = runtime("<div id=outer><a id=link href=/x>go</a></div><p id=log></p>");
        rt.exec(
            "const log = document.getElementById('log');
             document.getElementById('outer').addEventListener('click', e => { log.textContent += 'outer:' + e.target.id; });
             document.getElementById('link').addEventListener('click', e => { log.textContent += 'link,'; e.preventDefault(); });",
            "test",
        );
        let link = dom.borrow().find(0, |e| e.id() == Some("link"))[0];
        let prevented = rt.dispatch(Some(link), "click", EventInit::simple(true));
        assert!(prevented);
        assert_eq!(text_of(&dom, "log"), "link,outer:link");
    }

    #[test]
    fn timers_run_on_tick() {
        let (dom, mut rt) = runtime("<p id=out>0</p>");
        rt.exec(
            "setTimeout(() => { document.getElementById('out').textContent = 'done'; }, 0);",
            "test",
        );
        assert_eq!(text_of(&dom, "out"), "0");
        std::thread::sleep(Duration::from_millis(5));
        assert!(rt.tick());
        assert_eq!(text_of(&dom, "out"), "done");
    }

    #[test]
    fn location_and_navigation_requests() {
        let (_dom, mut rt) = runtime("<p></p>");
        rt.exec(
            "if (location.hostname === 'example.com') location.href = '/next?q=1';",
            "test",
        );
        let req = rt.take_requests();
        assert_eq!(req.navigate.unwrap().as_str(), "https://example.com/next?q=1");
    }
}
