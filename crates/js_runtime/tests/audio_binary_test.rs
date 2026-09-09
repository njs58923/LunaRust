use js_runtime::{Engine, FetchResponse};
fn settle(engine: &mut Engine) {
    for i in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        engine.fire_raf(i as f64 * 10.);
        engine
            .eval("if(globalThis.failure) throw Error(failure)")
            .unwrap();
        if engine
            .eval("if(!globalThis.done) throw Error('pending')")
            .is_ok()
        {
            return;
        }
    }
    panic!("Promise did not settle");
}
#[test]
fn binary_fetch_preserves_non_utf8_and_view_ranges() {
    let mut e = Engine::new();
    e.configure_document_location("https://audio.test/world".into());
    e.eval(r#"
      globalThis.done=false;globalThis.failure='';
      const raw=new Uint8Array([7,0,255,128,239,187,191,9]);
      fetch('/clip',{method:'POST',body:raw.subarray(1,7)}).then(async r=>{
        const copy=r.clone();const b=await r.arrayBuffer();
        if(!(b instanceof ArrayBuffer)||[...new Uint8Array(b)].join(',')!=='0,255,128,239,187,191')throw Error('binary corrupted');
        const blob=await copy.blob();if(blob.type!=='audio/wav'||blob.size!==6)throw Error('blob metadata');
        let twice=false;try{await r.text();}catch(e){twice=true;}if(!twice)throw Error('body reuse');done=true;
      }).catch(e=>failure=String(e));
    "#).unwrap();
    let (id, r) = e.drain_fetch_queue().pop().unwrap();
    assert_eq!(r.body_bytes, Some(vec![0, 255, 128, 239, 187, 191]));
    e.push_fetch_result(
        id,
        Ok(FetchResponse {
            url: r.url,
            status: 200,
            headers: vec![("content-type".into(), "audio/wav".into())],
            body: vec![0, 255, 128, 239, 187, 191],
            ..Default::default()
        }),
    );
    settle(&mut e);
}
#[test]
fn blobs_utf8_object_urls_and_bounded_io() {
    let mut e = Engine::new();
    e.configure_document_location("https://audio.test/world".into());
    e.eval(r#"
      globalThis.done=false;globalThis.failure='';
      (async()=>{
        const input=new Uint8Array([8,0,255,7]);const blob=new Blob([input.subarray(1,3),'ñ'],{type:'AUDIO/WAV'});input.fill(1);
        if([...(await blob.bytes())].join(',')!=='0,255,195,177')throw Error('blob did not snapshot view');
        if(await blob.slice(-2).text()!=='ñ')throw Error('blob slice');
        const url=URL.createObjectURL(blob);const r=await fetch(url);URL.revokeObjectURL(url);
        if((await r.arrayBuffer()).byteLength!==4)throw Error('revocation invalidated acquired response');
        let denied=false;try{await fetch(url);}catch(e){denied=true;}if(!denied)throw Error('revoked URL still accessible');
        const dest=new Uint8Array(3);const enc=new TextEncoder().encodeInto('ñ😀',dest);
        if(enc.read!==1||enc.written!==2)throw Error('encodeInto split a code point');
        const b=new IO.Buffer(3);if(IO.write(b,new Uint8Array([1,2,3,4]))!==3)throw Error('capacity');b.seek(0);
        const out=new Uint8Array(4);if(IO.read(b,out)!==3||IO.read(b,out)!==null)throw Error('buffer EOF');
        const {reader,writer}=IO.pipe({capacity:2});await writer.write(new Uint8Array([1,2]));
        let completed=false;const blocked=writer.write(new Uint8Array([3])).then(n=>{completed=true;return n;});
        await Promise.resolve();if(completed)throw Error('missing backpressure');
        const one=new Uint8Array(1);await reader.read(one);if(one[0]!==1)throw Error('pipe order');
        await blocked;writer.close();const tail=new Uint8Array(4);if(await reader.read(tail)!==2||tail[0]!==2||tail[1]!==3||await reader.read(tail)!==null)throw Error('pipe EOF/order');
        const p=IO.pipe({capacity:1});await p.writer.write(new Uint8Array([1]));const waiting=p.writer.write(new Uint8Array([2]));p.reader.close();
        let cancelled=false;try{await waiting;}catch(e){cancelled=true;}if(!cancelled)throw Error('cancel did not reject producer');
        done=true;
      })().catch(e=>failure=String(e));
    "#).unwrap();
    settle(&mut e);
}
#[test]
fn pcm_backpressure_eof_isolation_and_disposal() {
    let mut e = Engine::new();
    e.eval(r#"
      const ops=Deno.core.ops;
      globalThis.voice=ops.op_audio_create('stream',new Uint8Array(),8000,1,0.1);
      let rejected=false;try{ops.op_audio_write(voice,new Uint8Array(new Float32Array([NaN]).buffer));}catch(e){rejected=true;}if(!rejected)throw Error('NaN accepted');
      const data=new Float32Array(900).fill(0.25);
      if(ops.op_audio_write(voice,new Uint8Array(data.buffer))!==800)throw Error('PCM capacity');
      if(ops.op_audio_write(voice,new Uint8Array(data.buffer))!==0)throw Error('PCM backpressure');
      ops.op_audio_control(voice,'play',0);
    "#).unwrap();
    let p = e.drain_audio_commands().pop().unwrap();
    p.lock().unwrap().allowed = true;
    let mut decoder = js_runtime::audio::AudioDecoder::new(p.clone());
    for _ in 0..800 {
        assert_eq!(decoder.next(), Some(0.25));
    }
    for _ in 0..512 {
        assert_eq!(decoder.next(), Some(0.));
    }
    assert!(!p.lock().unwrap().ended, "starvation is not EOF");
    e.eval("ops.op_audio_control(voice,'end',0)").unwrap();
    for _ in 0..512 {
        decoder.next();
    }
    assert!(p.lock().unwrap().ended);
    let mut other = Engine::new();
    other.eval("let denied=false;try{Deno.core.ops.op_audio_control(1,'play',0)}catch(e){denied=true}if(!denied)throw Error('cross-isolate voice')").unwrap();
    drop(other);
    drop(e);
    for _ in 0..256 {
        decoder.next();
    }
    assert_eq!(decoder.next(), None);
}
#[test]
fn audio_stream_public_api_settles_play_and_disposes() {
    let mut e = Engine::new();
    e.eval("globalThis.done=false;globalThis.failure='';globalThis.s=new AudioStream({sampleRate:8000,channels:1,bufferSeconds:0.1});s.volume=0.4;s.play().then(()=>{s.pause();s.dispose();done=true}).catch(e=>failure=String(e))").unwrap();
    let p = e.drain_audio_commands().pop().unwrap();
    p.lock().unwrap().allowed = true;
    let _decoder = js_runtime::audio::AudioDecoder::new(p.clone());
    settle(&mut e);
    assert!(p.lock().unwrap().disposed);
}
#[test]
fn audio_rejects_invalid_data_and_bounds_voice_count() {
    let mut e = Engine::new();
    e.eval(r#"
      const ops=Deno.core.ops;let bad=false;
      try{ops.op_audio_create('clip',new Uint8Array([0,1,2]),0,0,0);}catch(e){bad=true;}if(!bad)throw Error('invalid decoder input');
      for(let i=0;i<16;i++)ops.op_audio_create('stream',new Uint8Array(),8000,1,0.1);
      let limited=false;try{ops.op_audio_create('stream',new Uint8Array(),8000,1,0.1);}catch(e){limited=true;}if(!limited)throw Error('voice limit');
    "#).unwrap();
}
#[test]
fn wav_clip_decodes_seeks_loops_and_uses_volume() {
    let mut bytes = Vec::new();
    let samples = [0i16, 16384, -16384, 8192];
    bytes.extend(b"RIFF");
    bytes.extend((36u32 + samples.len() as u32 * 2).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((samples.len() as u32 * 2).to_le_bytes());
    for s in samples {
        bytes.extend(s.to_le_bytes());
    }
    let mut e = Engine::new();
    e.eval(&format!("const ops=Deno.core.ops;const clip=ops.op_audio_create('clip',new Uint8Array({}),0,0,0);ops.op_audio_control(clip,'volume',0.5);ops.op_audio_control(clip,'loop',1);ops.op_audio_control(clip,'play',0);",serde_json::to_string(&bytes).unwrap())).unwrap();
    let p = e.drain_audio_commands().pop().unwrap();
    p.lock().unwrap().allowed = true;
    let mut decoder = js_runtime::audio::AudioDecoder::new(p.clone());
    for i in 0..256 {
        let expected = samples[i % 4] as f32 / 32768. * 0.5;
        assert!((decoder.next().unwrap() - expected).abs() < 0.0001);
    }
    e.eval("if(ops.op_audio_control(clip,'status',0).currentTime>=0.0005)throw Error('loop time not wrapped');ops.op_audio_control(clip,'seek',0.000125);ops.op_audio_control(clip,'loop',0)").unwrap();
    assert!((decoder.next().unwrap() - 0.25).abs() < 0.0001);
    for _ in 0..255 {
        decoder.next();
    }
    assert!(p.lock().unwrap().ended);
    e.eval("ops.op_audio_control(clip,'dispose',0)").unwrap();
    assert_eq!(decoder.next(), None);
}
