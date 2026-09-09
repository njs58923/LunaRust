(function(g){
  'use strict';
  const ops=Deno.core.ops;
  function freeze(v){if(v&&typeof v==='object'){for(const x of Object.values(v))freeze(x);Object.freeze(v);}return v;}
  function serialize(value,props=false){
    const seen=new Set();
    function visit(v,depth){
      if(depth>32)throw new TypeError('Component data exceeds depth 32');
      if(v===null||typeof v==='string'||typeof v==='boolean')return;
      if(typeof v==='number'&&Number.isFinite(v))return;
      if(typeof v!=='object')throw new TypeError('Component data must be JSON; functions and undefined cannot cross isolates');
      const proto=Object.getPrototypeOf(v);
      if(!Array.isArray(v)&&proto!==Object.prototype&&proto!==null)throw new TypeError('Component data must contain plain objects');
      if(seen.has(v))throw new TypeError('Cyclic component data');seen.add(v);
      if(Object.getOwnPropertySymbols(v).length)throw new TypeError('Symbol keys cannot cross isolates');
      if(Array.isArray(v)&&v.length>32768)throw new RangeError('Component array exceeds payload limit');
      const keys=Array.isArray(v)?Array.from({length:v.length},(_,i)=>String(i)):Object.keys(v);
      for(const key of keys){const d=Object.getOwnPropertyDescriptor(v,key);if(!d||!('value' in d))throw new TypeError('Sparse arrays and accessors cannot cross isolates');visit(d.value,depth+1);}
      seen.delete(v);
    }
    if(props&&(!value||Array.isArray(value)||typeof value!=='object'))throw new TypeError('Props must be a plain object');
    visit(value,0);const text=JSON.stringify(value);
    if(new TextEncoder().encode(text).length>65536)throw new RangeError('Component payload exceeds 64 KiB');return text;
  }
  const optimistic=new WeakMap();
  Object.defineProperty(HSMLElement.prototype,'props',{
    get(){
      if(this.tagName!=='include')throw new TypeError('props belongs to include elements');
      const raw=this.getAttribute('props')||'{}', pending=optimistic.get(this);
      if(pending&&raw===pending.base)return pending.value;
      optimistic.delete(this);
      const native=this._isResolved()?ops.op_component_props(this.nodeId):null;
      return freeze(native??JSON.parse(raw));
    },
    set(value){
      if(this.tagName!=='include')throw new TypeError('props belongs to include elements');
      const text=serialize(value,true);const base=this.getAttribute('props')||'{}';
      optimistic.set(this,{base,value:freeze(JSON.parse(text))});this.setAttribute('props',text);
    }
  });
  const listeners=new Map();let observed=null, cached=null;
  function read(){const s=ops.op_component_context();if(!cached||cached.generation!==s.generation||cached.revision!==s.revision||cached.connected!==s.connected)cached=freeze(s);if(!observed)observed=cached;return cached;}
  function notify(type,detail){const event=Object.freeze({type,detail,target:api,isTrusted:false});for(const fn of [...(listeners.get(type)||[])]){try{fn(event);}catch(e){console.error(e);}}}
  const api=Object.freeze({
    get props(){return read().props;},get connected(){return read().connected;},get revision(){return read().revision;},
    addEventListener(type,fn){if(typeof fn!=='function')return;if(!listeners.has(type))listeners.set(type,new Set());listeners.get(type).add(fn);read();},
    removeEventListener(type,fn){listeners.get(type)?.delete(fn);},
    async emit(type,detail=null){ops.op_component_emit(String(type),serialize(detail));}
  });
  Object.defineProperty(g,'component',{value:api});
  g.__luna_component_pump=function(){
    const previous=observed;const current=read();observed=current;
    if(previous&&previous!==current){
      if(previous.connected&&!current.connected)notify('disconnect',{});
      else if(current.connected){
        if(!previous.connected||previous.generation!==current.generation)notify('connect',{});
        const keys=new Set([...Object.keys(previous.props),...Object.keys(current.props)]);
        const changedKeys=[...keys].filter(k=>JSON.stringify(previous.props[k])!==JSON.stringify(current.props[k]));
        notify('propschange',freeze({props:current.props,previous:previous.props,revision:current.revision,changedKeys}));
      }
    }
    for(const event of ops.op_component_poll()){
      if(!ops.op_component_validate(event.nodeId,event.generation))continue;
      event.detail=freeze(event.detail);event.isTrusted=false;event.bubbles=false;
      g.__luna_dispatch_dom_events([event]);
    }
  };
})(globalThis);
