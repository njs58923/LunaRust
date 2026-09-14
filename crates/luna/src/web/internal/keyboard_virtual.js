// Spanish keyboard adapted from server_hsml/public/test/teclado.
// Retained keys; only active hover/press transitions request animation frames.
(function() {
  const root=hiperspace.dimention, group=root.getElementById('keys');
  let symbols=false, shift=false, frame=null;
  const entries=[], active=new Set();
  const labels={Copy:'Copiar',Cut:'Cortar',Paste:'Pegar',Backspace:'⌫',Enter:'Enter',ArrowLeft:'←',ArrowRight:'→',Home:'Inicio',End:'Fin',Close:'Cerrar',Shift:'⇧',' ':'Espacio'};
  function emit(type,data) { component.emit(type,data).catch(e=>console.warn('[keyboard]',String(e))); }
  function press(key) {
    if(key==='Shift'){shift=!shift;refresh();return;}
    if(key==='123'){symbols=!symbols;refresh();return;}
    if(key==='Close'){emit('close',{revision:component.props.revision});return;}
    const shortcut=({Copy:'c',Cut:'x',Paste:'v'})[key];
    const printable=Array.from(key).length===1;
    const value=shortcut||(printable&&shift?key.toUpperCase():key);
    const code=value===' '?'Space':/^[a-z]$/i.test(value)?'Key'+value.toUpperCase():/^[0-9]$/.test(value)?'Digit'+value:printable?'Unidentified':value;
    for(const type of ['keydown','keyup'])emit('key',{revision:component.props.revision,packet:{type,key:value,code,text:type==='keydown'&&printable?value:'',repeat:false,shiftKey:shift,ctrlKey:!!shortcut,altKey:false,metaKey:false,isComposing:false}});
    if(shift&&printable){shift=false;refresh();}
  }
  function mix(a,b,t) {
    return '#'+a.map((v,i)=>Math.round(v+(b[i]-v)*t).toString(16).padStart(2,'0')).join('');
  }
  function paint(e,now) {
    const desired=e.pointers.size?1:0;
    const t=Math.min(1,(now-e.started)/120), eased=1-Math.pow(1-t,3);
    e.hover=e.from+(desired-e.from)*eased;
    const pulse=Math.max(0,1-(now-e.pressed)/170);
    const push=Math.sin(pulse*Math.PI/2);
    const selected=(e.key==='Shift'&&shift)||(e.key==='123'&&symbols);
    const base=e.key==='Close'?[120,62,72]:selected?[58,91,145]:[48,59,77];
    const color=mix(base,[106,156,225],Math.min(1,e.hover*0.55+push*0.45));
    const scale=1+e.hover*0.025-push*0.025;
    const z=0.018+e.hover*0.004-push*0.006;
    e.box.setAttribute('color',color);
    e.box.setAttribute('sx',String(e.width*scale));e.box.setAttribute('sy',String(0.09*scale));
    e.box.setAttribute('z',String(z));e.text.setAttribute('z',String(z+0.010));
    e.text.setAttribute('size',String(e.size*scale));
    return t<1 || pulse>0;
  }
  function tick() {
    frame=null;const now=Date.now();
    for(const e of active)if(!e.visible||!paint(e,now))active.delete(e);
    if(active.size)frame=requestAnimationFrame(tick);
  }
  function animate(e) {
    e.from=e.hover;e.started=Date.now();active.add(e);
    if(frame===null)frame=requestAnimationFrame(tick);
  }
  function createKey() {
    const box=root.createElement('box'),text=root.createElement('text');
    group.appendChild(box);group.appendChild(text);
    const e={box,text,pointers:new Set(),hover:0,from:0,started:0,pressed:-Infinity,visible:true};
    for(const[k,v]of Object.entries({sz:0.012,'border-radius':0.009,touchable:true,'pointer-blocking':true}))box.setAttribute(k,String(v));
    text.setAttribute('color','#FFFFFF');text.setAttribute('touchable','false');
    box.addEventListener('pointerenter',event=>{e.pointers.add(event.pointerId||0);animate(e);});
    box.addEventListener('pointerleave',event=>{e.pointers.delete(event.pointerId||0);animate(e);});
    box.addEventListener('toque',()=>{if(e.visible){e.pressed=Date.now();animate(e);press(e.key);}});
    return e;
  }
  function refresh() {
    const rows=symbols?
      ['1234567890'.split('').concat('Backspace'),'@#$%&-+()/'.split('').concat('Enter'),['123',...':;"\'!?_=[]'.split('')],['Tab','Home','ArrowLeft',' ','ArrowRight','End','Close']]:
      ['qwertyuiop'.split('').concat('Backspace'),'asdfghjklñ'.split('').concat('Enter'),['Shift',...'zxcvbnm,./'.split(''),'123'],['Tab','Home','ArrowLeft',' ','ArrowRight','End','Close']];
    rows[3].splice(1,0,'Copy','Cut','Paste');
    let index=0;
    rows.forEach((row,r)=>{
      const weights=row.map(k=>k===' '?4:k.length>1?1.4:1),total=weights.reduce((a,b)=>a+b,0);
      let x=-0.565;
      row.forEach((key,i)=>{
        const e=entries[index]||(entries[index]=createKey());index++;
        e.key=key;e.width=weights[i]/total*1.13-0.005;e.visible=true;
        const label=labels[key]||(key==='123'?(symbols?'ABC':'123'):(shift?key.toUpperCase():key));
        e.size=label.length>3?0.019:0.03;
        for(const node of [e.box,e.text]) {
          node.setAttribute('visible','inherit');node.setAttribute('x',String(x+e.width/2));node.setAttribute('y',String(0.165-r*0.105));
        }
        e.text.setAttribute('value',label);paint(e,Date.now());
        x+=e.width+0.005;
      });
    });
    for(let i=index;i<entries.length;i++) {
      const e=entries[i];e.visible=false;e.pointers.clear();e.hover=0;active.delete(e);
      e.box.setAttribute('visible','false');e.text.setAttribute('visible','false');
    }
  }
  component.addEventListener('disconnect',()=>{if(frame!==null)cancelAnimationFrame(frame);frame=null;active.clear();});
  refresh();
})();
