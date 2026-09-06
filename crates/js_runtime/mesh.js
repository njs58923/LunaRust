(() => {
  const op = Deno.core.ops.op_mesh_resource;
  const empty = new Uint8Array(0);
  function bytes(value, Type, name) {
    if (value == null) return empty;
    if (!(value instanceof Type)) {
      if (!Array.isArray(value)) throw new TypeError(name + ' must be an array or ' + Type.name);
      if (Type === Uint32Array && value.some(n => !Number.isInteger(n) || n < 0 || n > 0xffffffff)) {
        throw new RangeError('indices must be unsigned integers');
      }
      value = new Type(value);
    }
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  function submit(action, src, data = {}) {
    const result = op(action, src, bytes(data.positions, Float32Array, 'positions'),
      bytes(data.indices, Uint32Array, 'indices'), bytes(data.normals, Float32Array, 'normals'),
      bytes(data.uvs, Float32Array, 'uvs'), bytes(data.colors, Float32Array, 'colors'));
    if (result.error) throw new Error(result.error);
    return result.src;
  }
  globalThis.MeshResource = Object.freeze({
    create(data) {
      const src = submit('create', '', data);
      let disposed = false;
      return Object.freeze({
        src,
        update(data) {
          if (disposed) throw new Error('Mesh resource disposed');
          submit('update', src, data);
        },
        dispose() {
          if (!disposed) { submit('dispose', src); disposed = true; }
        },
        toString() { return src; }
      });
    }
  });
})();
