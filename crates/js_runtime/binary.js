(function(g) {
  'use strict';
  const ops = Deno.core.ops;
  const MAX = 8 * 1024 * 1024;
  function view(value) {
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    throw new TypeError('Expected ArrayBuffer or an ArrayBuffer view');
  }
  class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') { return ops.op_encode_utf8(String(input)); }
    encodeInto(input, destination) {
      if (!(destination instanceof Uint8Array)) throw new TypeError('Expected Uint8Array');
      let read = 0, written = 0;
      for (const char of String(input)) {
        const encoded = this.encode(char);
        if (written + encoded.length > destination.length) break;
        destination.set(encoded, written); written += encoded.length; read += char.length;
      }
      return {read,written};
    }
  }
  class TextDecoder {
    constructor(label='utf-8', options={}) {
      if (!['utf-8','utf8','unicode-1-1-utf-8'].includes(String(label).trim().toLowerCase())) throw new RangeError('Only UTF-8 is supported');
      this.fatal = !!options.fatal; this.ignoreBOM = !!options.ignoreBOM;
    }
    get encoding() { return 'utf-8'; }
    decode(input=new Uint8Array(), options={}) {
      if (options.stream) throw new TypeError('Incremental text decoding is not implemented');
      return ops.op_decode_utf8(view(input), this.fatal, this.ignoreBOM);
    }
  }
  const encoder = new TextEncoder(), decoder = new TextDecoder();
  const blobs = new WeakMap();
  class Blob {
    constructor(parts=[], options={}) {
      const chunks = [];
      let length = 0;
      for (const part of parts) {
        const bytes = part instanceof Blob ? blobs.get(part) :
          (part instanceof ArrayBuffer || ArrayBuffer.isView(part)) ? view(part) : encoder.encode(String(part));
        length += bytes.length;
        if (length > MAX) throw new RangeError('Blob exceeds 8 MiB');
        chunks.push(bytes);
      }
      const bytes = new Uint8Array(length); let offset=0;
      for (const chunk of chunks) { bytes.set(chunk,offset); offset += chunk.length; }
      blobs.set(this,bytes);
      const type = String(options.type || '');
      Object.defineProperties(this,{size:{value:length,enumerable:true},type:{value:/[^\x20-\x7e]/.test(type)?'':type.toLowerCase(),enumerable:true}});
    }
    async arrayBuffer() { return blobs.get(this).slice().buffer; }
    async bytes() { return blobs.get(this).slice(); }
    async text() { return decoder.decode(blobs.get(this)); }
    slice(start=0,end=this.size,type='') { return new Blob([blobs.get(this).slice(start,end)],{type}); }
    get [Symbol.toStringTag]() { return 'Blob'; }
  }
  const urls=new Map(); let nextUrl=0, urlBytes=0;
  // Identity only, not an authorization token: resolution remains isolate-local.
  const urlNamespace=Date.now().toString(36)+'-'+Math.random().toString(36).slice(2)+'-'+Math.random().toString(36).slice(2);
  URL.createObjectURL = blob => {
    if (!(blob instanceof Blob)) throw new TypeError('Expected Blob');
    if (urls.size >= 128 || urlBytes + blob.size > 64*1024*1024) throw new RangeError('Object URL quota exceeded');
    const url='blob:'+location.origin+'/luna-'+urlNamespace+'-'+(++nextUrl);
    urls.set(url,blob); urlBytes+=blob.size; return url;
  };
  URL.revokeObjectURL = url => { const b=urls.get(String(url)); if(b) {urlBytes-=b.size;urls.delete(String(url));} };
  // Private, isolate-local bridge shared by fetch/audio; never a global blob registry.
  Object.defineProperty(g,'__lunaBinary',{value:Object.freeze({view, blobBytes:b=>blobs.get(b), resolveBlob:url=>urls.get(String(url))})});
  class ByteBuffer {
    constructor(input=0) {
      const bytes=typeof input==='number'?null:view(input);
      const capacity=bytes ? bytes.length : input;
      if(!Number.isInteger(capacity)||capacity<0||capacity>MAX) throw new RangeError('Buffer capacity must be 0..8 MiB');
      this._bytes=new Uint8Array(capacity); if(bytes)this._bytes.set(bytes);
      this.position=0; this.length=bytes?bytes.length:0; this.closed=false;
    }
    _check() { if(this.closed)throw new Error('Buffer closed'); }
    read(destination) {
      this._check(); const out=view(destination);
      if(!out.length)return 0;
      const n=Math.min(out.length,this.length-this.position); if(n<=0)return null;
      out.set(this._bytes.subarray(this.position,this.position+n)); this.position+=n; return n;
    }
    write(source) {
      this._check();const bytes=view(source),n=Math.min(bytes.length,this._bytes.length-this.position);
      this._bytes.set(bytes.subarray(0,n),this.position);this.position+=n;this.length=Math.max(this.length,this.position);return n;
    }
    seek(position) { this._check();if(!Number.isInteger(position)||position<0||position>this.length)throw new RangeError('Invalid position');this.position=position;return position; }
    bytes() {this._check();return this._bytes.slice(0,this.length);}
    close() {this.closed=true;this._bytes=new Uint8Array();}
  }
  function pipe({capacity=65536}={}) {
    if(!Number.isInteger(capacity)||capacity<1||capacity>MAX)throw new RangeError('Invalid pipe capacity');
    const ring=new Uint8Array(capacity);let start=0,length=0,ended=false,cancelled=false;
    let pendingRead=null,pendingWrite=null;
    function pump() {
      if(cancelled)return;
      if(pendingRead && (length || ended || !pendingRead.bytes.length)) {
        const r=pendingRead;pendingRead=null;
        const n=Math.min(r.bytes.length,length);
        for(let i=0;i<n;i++)r.bytes[i]=ring[(start+i)%capacity];
        start=(start+n)%capacity;length-=n;r.resolve(n||(!r.bytes.length?0:null));
      }
      if(pendingWrite && length<capacity) {
        const w=pendingWrite;pendingWrite=null;const n=Math.min(w.bytes.length,capacity-length);
        for(let i=0;i<n;i++)ring[(start+length+i)%capacity]=w.bytes[i];
        length+=n;w.resolve(n);pump();
      }
    }
    const reader={read(destination){
      if(cancelled)return Promise.reject(new Error('Pipe cancelled'));
      if(pendingRead)return Promise.reject(new Error('Only one pending read is allowed'));
      return new Promise((resolve,reject)=>{pendingRead={bytes:view(destination),resolve,reject};pump();});
    },close(){cancelled=true;length=0;for(const p of [pendingRead,pendingWrite])if(p)p.reject(new Error('Pipe cancelled'));pendingRead=pendingWrite=null;}};
    const writer={write(source){
      if(ended||cancelled)return Promise.reject(new Error('Pipe closed'));
      if(pendingWrite)return Promise.reject(new Error('Only one pending write is allowed'));
      const bytes=view(source);if(!bytes.length)return Promise.resolve(0);
      // Copy only the portion that can be queued, not an unbounded input.
      return new Promise((resolve,reject)=>{pendingWrite={bytes:bytes.slice(0,capacity),resolve,reject};pump();});
    },close(){ended=true;if(pendingWrite){pendingWrite.reject(new Error('Pipe closed'));pendingWrite=null;}pump();}};
    return {reader,writer};
  }
  g.TextEncoder=TextEncoder;g.TextDecoder=TextDecoder;g.Blob=Blob;
  g.IO=Object.freeze({Buffer:ByteBuffer,pipe,read:(r,b)=>r.read(b),write:(w,b)=>w.write(b)});
})(globalThis);
