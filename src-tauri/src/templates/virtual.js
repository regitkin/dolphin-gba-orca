const SLOT={slot};

let ws=null;
let prevBuf=null;

const buttonIndices={
  a:      0,
  b:      1,
  l:      4,
  r:      5,
  select: 8,
  start:  9,
  up:     12,
  down:   13,
  left:   14,
  right:  15
};

function connectVirtualController(){
  if(ws&&(ws.readyState===0||ws.readyState===1)) return;

  try{
    ws=new WebSocket('ws://'+location.host+'/ws/'+SLOT);
  }catch(e){
    ws=null;
    return;
  }

  ws.onopen=()=>{
    prevBuf=null;
    sendState();
  };

  ws.onclose=()=>{
    ws=null;
    prevBuf=null;
  };

  ws.onerror=()=>{
    try{
      if(ws) ws.close();
    }catch(e){}
  };
}

function bufEqual(a,b){
  if(!a||!b||a.length!==b.length) return false;
  for(let i=0;i<a.length;i++){
    if(a[i]!==b[i]) return false;
  }
  return true;
}

function sendState(){
  if(!ws||ws.readyState!==1) return;

  const buf=new Uint8Array(32);

  // Axes remain zero for the virtual controller.
  // Buttons occupy bytes 16..31, matching Web Gamepad API indices.
  
  if(!bufEqual(buf,prevBuf)){
    ws.send(buf);
    prevBuf=buf;
  }
}

function setButton(name,pressed){
  if(!ws||ws.readyState!==1) return;

  const index=buttonIndices[name];
  if(index===undefined) return;

  // Web Gamepad buttons begin at byte 16.
  const buf=prevBuf
    ? new Uint8Array(prevBuf)
    : new Uint8Array(32);

  buf[16+index]=pressed?1:0;

  if(!bufEqual(buf,prevBuf)){
    ws.send(buf);
    prevBuf=buf;
  }
}

function bindButton(button){
  const name=button.dataset.button;
  if(!name) return;

  const press=e=>{
    e.preventDefault();

    if(!ws||ws.readyState!==1){
      connectVirtualController();
      return;
    }

    button.classList.add('pressed');
    setButton(name,true);
  };

  const release=e=>{
    e.preventDefault();

    button.classList.remove('pressed');

    if(ws&&ws.readyState===1){
      setButton(name,false);
    }
  };

  button.addEventListener('pointerdown',press);
  button.addEventListener('pointerup',release);
  button.addEventListener('pointercancel',release);
  button.addEventListener('pointerleave',release);

  button.addEventListener('contextmenu',e=>e.preventDefault());
}

// Groups buttons so that you can slide between them without needing to lift your finger/left mouse button.
// requires data-hit elements that act as hit-boxes for the real buttons
function bindButtonGroup(group){
  let activePointerId=null;
  let current=null; // {name, visual}

  // Gets the element that the cursor is on, based on data-hit hitboxes
  function resolveAt(x,y){
    const el=document.elementFromPoint(x,y);
    if(!el) return null;

    const hitEl=el.closest && el.closest('[data-hit]');
    if(hitEl && group.contains(hitEl)){
      const name=hitEl.dataset.hit;
      const visual=group.querySelector('[data-button="'+name+'"]') || hitEl;
      return {name, visual};
    }

    return null;
  }

  function press(control){
    if(!control || (current && current.name===control.name)) return;
    release();
    current=control;
    control.visual.classList.add('pressed');
    setButton(control.name,true);
  }

  function release(){
    if(!current) return;
    current.visual.classList.remove('pressed');
    setButton(current.name,false);
    current=null;
  }

  group.addEventListener('pointerdown',e=>{
    e.preventDefault();
    const control=resolveAt(e.clientX,e.clientY);
    if(!control) return;

    activePointerId=e.pointerId;
    try{ group.setPointerCapture(e.pointerId); }catch(err){}

    if(!ws||ws.readyState!==1){
      connectVirtualController();
    }
    press(control);
  });

  group.addEventListener('pointermove',e=>{
    if(e.pointerId!==activePointerId) return;
    const control=resolveAt(e.clientX,e.clientY);
    if(control){
      press(control);
    }else{
      // User still pressing but no longer on a button
      release();
    }
  });

  function endPointer(e){
    if(e.pointerId!==activePointerId) return;
    release();
    try{ group.releasePointerCapture(e.pointerId); }catch(err){}
    activePointerId=null;
  }

  group.addEventListener('pointerup',endPointer);
  group.addEventListener('pointercancel',endPointer);
  group.addEventListener('pointerleave',e=>{
    if(e.pointerId!==activePointerId) return;
    endPointer(e);
  });

  group.addEventListener('contextmenu',e=>e.preventDefault());
}

const dpadEl=document.querySelector('#virtual-controller .vc-dpad');
const faceEl=document.querySelector('#virtual-controller .vc-face');
const groupedContainers=[dpadEl,faceEl];

document
  .querySelectorAll('#virtual-controller [data-button]')
  .forEach(button=>{
    if(groupedContainers.some(g=>g && g.contains(button))) return;
    bindButton(button);
  });

if(dpadEl) bindButtonGroup(dpadEl);
if(faceEl) bindButtonGroup(faceEl);

connectVirtualController();
