(function(g) {
  'use strict';
  const ops=Deno.core.ops, empty=new Uint8Array();
  const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  class Voice {
    constructor(){this._id=0;this._disposed=false;this._volume=1;this._muted=false;this._loop=false;this._events=new Map();this._timer=null;this._playEpoch=0;}
    addEventListener(type,callback){if(typeof callback==='function'){if(!this._events.has(type))this._events.set(type,new Set());this._events.get(type).add(callback);}}
    removeEventListener(type,callback){this._events.get(type)?.delete(callback);}
    _emit(type,error){const event={type,target:this,error};for(const fn of [...(this._events.get(type)||[])]){try{fn.call(this,event);}catch(e){console.error(e);}}const fn=this['on'+type];if(typeof fn==='function'){try{fn.call(this,event);}catch(e){console.error(e);}}}
    _command(action,value=0){if(this._disposed)throw new Error('Audio disposed');if(!this._id)throw new Error('Audio not loaded');return ops.op_audio_control(this._id,action,value);}
    async _waitReady(id){
      const start=Date.now();
      while(true){
        if(this._disposed||this._id!==id)throw new Error('Audio load cancelled');
        const s=this._command('status');if(s.error)throw new Error(s.error);if(s.ready)return;
        if(Date.now()-start>5000)throw new Error('Audio output unavailable (no active mixer/device)');
        await delay(10);
      }
    }
    _watch(){
      if(this._timer!==null)return;
      const poll=()=>{
        this._timer=null;if(this._disposed||!this._id)return;
        try{const s=this._command('status');if(s.error){this._emit('error',new Error(s.error));return;}this._emit('timeupdate');if(s.ended){this._emit('ended');return;}if(!s.paused)this._timer=setTimeout(poll,50);}
        catch(e){this._emit('error',e);}
      };
      this._timer=setTimeout(poll,50);
    }
    pause(){this._playEpoch++;if(this._id)this._command('pause');if(this._timer!==null){clearTimeout(this._timer);this._timer=null;}this._emit('pause');}
    get paused(){return !this._id||this._command('status').paused;}
    get ended(){return !!this._id&&this._command('status').ended;}
    get currentTime(){return this._id?this._command('status').currentTime:0;}
    get duration(){return this._id?(this._command('status').duration??Infinity):NaN;}
    get volume(){return this._volume;}
    set volume(value){value=Number(value);if(!Number.isFinite(value)||value<0||value>1)throw new RangeError('Volume must be 0..1');this._volume=value;if(this._id)this._command('volume',this._muted?0:value);}
    get muted(){return this._muted;}
    set muted(value){this._muted=!!value;if(this._id)this._command('volume',this._muted?0:this._volume);}
    _release(){if(this._timer!==null)clearTimeout(this._timer);this._timer=null;if(this._id){ops.op_audio_control(this._id,'dispose',0);this._id=0;}}
    dispose(){if(this._disposed)return;this._playEpoch++;this._release();this._disposed=true;this._events.clear();}
  }
  class Audio extends Voice {
    constructor(src=''){super();this._src=src;this._generation=0;this._loading=null;}
    get src(){return this._src;}
    set src(value){if(this._disposed)throw new Error('Audio disposed');this._generation++;this._playEpoch++;this._release();this._loading=null;this._src=value;}
    get loop(){return this._loop;}
    set loop(value){this._loop=!!value;if(this._id)this._command('loop',this._loop?1:0);}
    get currentTime(){return super.currentTime;}
    set currentTime(value){this._command('seek',Number(value));}
    load(){
      if(this._disposed)return Promise.reject(new Error('Audio disposed'));
      if(this._loading)return this._loading;
      if(this._id)return Promise.resolve();
      const generation=this._generation;
      this._loading=(async()=>{
        let bytes;
        if(this._src instanceof Blob)bytes=new Uint8Array(await this._src.arrayBuffer());
        else if(this._src instanceof ArrayBuffer||ArrayBuffer.isView(this._src))bytes=__lunaBinary.view(this._src);
        else {if(!this._src)throw new TypeError('Audio source is empty');const response=await fetch(String(this._src));if(!response.ok)throw new Error('Audio HTTP '+response.status);bytes=new Uint8Array(await response.arrayBuffer());}
        if(this._disposed||generation!==this._generation)throw new Error('Audio load cancelled');
        const id=ops.op_audio_create('clip',bytes,0,0,0);this._id=id;
        this._command('volume',this._muted?0:this._volume);this._command('loop',this._loop?1:0);
        await this._waitReady(id);this._emit('loadeddata');
      })().catch(e=>{if(generation===this._generation&&!this._disposed){this._release();this._emit('error',e);}throw e;}).finally(()=>{if(generation===this._generation)this._loading=null;});
      this._loading.catch(()=>{});return this._loading;
    }
    async play(){const epoch=++this._playEpoch;await this.load();if(epoch!==this._playEpoch)throw new Error('Audio play cancelled');this._command('play');this._emit('play');this._watch();}
    stop(){this.pause();if(this._id)this.currentTime=0;}
  }
  class AudioStream extends Voice {
    constructor({sampleRate=48000,channels=2,bufferSeconds=2}={}){
      super();
      if(!Number.isInteger(sampleRate)||!Number.isInteger(channels))throw new TypeError('Invalid PCM format');
      this._id=ops.op_audio_create('stream',empty,sampleRate,channels,bufferSeconds);
      Object.defineProperties(this,{sampleRate:{value:sampleRate},channels:{value:channels}});
      this._ready=this._waitReady(this._id).catch(e=>{this._release();this._emit('error',e);throw e;});this._ready.catch(()=>{});
    }
    get ready(){return this._ready;}
    async play(){const epoch=++this._playEpoch;await this.ready;if(epoch!==this._playEpoch)throw new Error('Audio play cancelled');this._command('play');this._emit('play');this._watch();}
    get bufferedFrames(){return this._command('status').bufferedFrames;}
    get writableFrames(){return this._command('status').writableFrames;}
    write(samples){
      if(this._disposed)throw new Error('Audio disposed');
      if(!(samples instanceof Float32Array)||samples.length%this.channels)throw new TypeError('Expected interleaved Float32Array with complete frames');
      return ops.op_audio_write(this._id,__lunaBinary.view(samples));
    }
    async writeAll(samples){
      if(this._writing)throw new Error('Only one writeAll may be pending');this._writing=true;
      try{let frame=0;while(frame*this.channels<samples.length){const n=this.write(samples.subarray(frame*this.channels));frame+=n;if(!n)await delay(10);}return frame;}finally{this._writing=false;}
    }
    end(){this._command('end');}
  }
  g.Audio=Audio;g.AudioStream=AudioStream;
})(globalThis);
