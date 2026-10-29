// Rendering formulas adapted from collidingScopes/ascii (Alan Ang, MIT),
// af6b92f7c48e2b46f7f29d18ba8409a0f93900ff. Offline preview only.
const $=id=>document.getElementById(id),canvas=$('art'),surface=document.createElement('canvas');surface.width=canvas.width;surface.height=canvas.height;
const ctx=surface.getContext('2d',{alpha:false}),display=canvas.getContext('2d',{alpha:false});
const ramp='____``..--^^~~<>??123456789%%&&@@'.replaceAll('_',' ');
const defaults={speed:.5,glyphSpeed:12,smooth:true,gradient:true,saturation:60,fontSize:3,resolution:70,threshold:30,randomness:15,invert:false,accent:'#feca17',secondary:'#6be8b9'};
let settings={...defaults};try{settings={...defaults,...JSON.parse(null||'{}')}}catch{}
const controls=[['speed','Движение анимации',0,2,.05,'×'],['smooth','Сглаживание движения'],['glyphSpeed','Обновление символов',0,30,1,'/с'],['gradient','Bg Gradient'],['saturation','Bg Saturation',0,100,1,'%'],['fontSize','Font Size Factor',.5,6,.1,''],['resolution','Resolution',20,140,1,''],['threshold','Threshold',0,95,1,'%'],['randomness','Randomness',0,100,1,'%'],['invert','Invert']];
for(const [id,label,min,max,step,unit] of controls){const boolean=min===undefined;const wrap=document.createElement('div');wrap.innerHTML=boolean?`<label>${label}<input id="${id}" type="checkbox" aria-label="${label}"></label>`:`<label for="${id}">${label}<output id="${id}Label"></output></label><input id="${id}" type="range" min="${min}" max="${max}" step="${step}" aria-label="${label}">`;$('settings').append(wrap);$(id).oninput=()=>{settings[id]=boolean?$(id).checked:Number($(id).value);glyphKey='';lastFrame=-1;sync();makePalette()}}
function sync(){delete settings.darkFill;delete settings.cover;delete settings.brushRadius;delete settings.brushStrength;delete settings.recovery;for(const [id,label,min,max,step,unit] of controls){if(min===undefined)$(id).checked=settings[id];else{$(id).value=settings[id];$(id+'Label').value=id==='speed'?'×'+settings[id].toFixed(2):settings[id]+unit}}$('accent').value=settings.accent;$('secondary').value=settings.secondary;$('accentHex').value=settings.accent;$('secondaryHex').value=settings.secondary;$('summary').textContent=JSON.stringify(settings,null,2);try{localStorage.setItem('ascii-preview-settings-v3',JSON.stringify(settings))}catch{}}
let meta,frames,palette,playing=true,position=0,lastTime=null,lastFrame=-1,raf=0,glyphTime=0,glyphKey='',glyphs=[];
$('fullscreen').onclick=()=>canvas.requestFullscreen().catch(()=>{$('fullscreen').textContent='Полный экран недоступен'});
document.addEventListener('fullscreenchange',()=>{canvas.width=document.fullscreenElement===canvas?screen.width:1440;canvas.height=document.fullscreenElement===canvas?screen.height:1088;surface.width=canvas.width;surface.height=canvas.height;canvas.style.width=document.fullscreenElement===canvas?'100vw':'100%';canvas.style.height=document.fullscreenElement===canvas?'100vh':'auto';glyphKey='';lastFrame=-1;draw()});
function rgb(hex){return [1,3,5].map(i=>parseInt(hex.slice(i,i+2),16))}
function hue(hex){let [r,g,b]=rgb(hex).map(v=>v/255),mx=Math.max(r,g,b),mn=Math.min(r,g,b),d=mx-mn;if(!d)return 210;return ((mx===r?(g-b)/d:mx===g?(b-r)/d+2:(r-g)/d+4)*60+360)%360}
function hslRgb(h,s,l){s/=100;l/=100;const a=s*Math.min(l,1-l);return [0,8,4].map(n=>{const k=(n+h/30)%12;return Math.round(255*(l-a*Math.max(-1,Math.min(k-3,9-k,1))))})}
function css(c){return `rgb(${c.join(',')})`}
function makePalette(){palette={bgHue:220,bg:'#080b10',a:rgb(settings.accent),b:rgb(settings.secondary)};$('bgChip').style.background=palette.bg;$('fontChip').style.background=css(palette.a);$('font2Chip').style.background=css(palette.b);lastFrame=-1;if(frames)draw()}
function noise(n){n=Math.imul(n^(n>>>16),0x45d9f3b);n=Math.imul(n^(n>>>16),0x45d9f3b);return ((n^(n>>>16))>>>0)/4294967295}
// Detect only persistent black borders across the entire animation, never per frame.
// Cache by frame buffer so importing another video recalculates the bounds.
let boundsFrames=null,bounds=null;
function contentBounds(){
 if(boundsFrames===frames)return bounds;
 const w=meta.columns,h=meta.rows,size=w*h,occupied=new Uint8Array(size);
 for(let i=0;i<frames.length;i++)if(frames[i]>8)occupied[i%size]=1;
 let left=w,top=h,right=-1,bottom=-1;
 for(let y=0;y<h;y++)for(let x=0;x<w;x++)if(occupied[y*w+x]){left=Math.min(left,x);right=Math.max(right,x);top=Math.min(top,y);bottom=Math.max(bottom,y)}
 bounds=right<0?{x:0,y:0,width:w,height:h}:{x:left,y:top,width:right-left+1,height:bottom-top+1};boundsFrames=frames;return bounds;
}
function draw(){if(!meta||!frames)return;let frame=Math.floor(position)%meta.frames;const sample=settings.smooth?position:frame,glyphTick=Math.floor(glyphTime),key=`${glyphTick}:${settings.resolution}`;if(sample===lastFrame&&key===glyphKey)return;lastFrame=sample;
 const size=meta.columns*meta.rows,offset=frame*size,nextOffset=((frame+1)%meta.frames)*size,blend=settings.smooth?position-Math.floor(position):0,pixel=Math.ceil(Math.min(canvas.width,canvas.height)/settings.resolution),cols=Math.ceil(canvas.width/pixel),rows=Math.ceil(canvas.height/pixel),randomness=settings.randomness/100,threshold=Math.max(0,Math.min(.98,settings.threshold/100+.2*Math.sin(glyphTime/30)*randomness));
 const updateGlyphs=key!==glyphKey;glyphKey=key;
 ctx.fillStyle=palette.bg;ctx.fillRect(0,0,canvas.width,canvas.height);ctx.font=`${pixel/.65*settings.fontSize/3}px 'Courier New',monospace`;ctx.textBaseline='alphabetic';
 const bg=[],fg=[];for(let v=0;v<256;v++){let g=v/255;const lightness=g>threshold?(settings.invert?1-(1-g)**2:g*g)*100:threshold/4*100;bg[v]=settings.gradient?`hsl(${palette.bgHue} ${settings.saturation}% ${lightness}%)`:palette.bg;const t=Math.max(0,Math.min(1,(g-threshold)/(1-threshold)));fg[v]=css(palette.a.map((c,i)=>Math.round(c+(palette.b[i]-c)*t)))}
 const crop=contentBounds(),sourceWidth=crop.width*meta.width/meta.columns,sourceHeight=crop.height*meta.height/meta.rows,scale=Math.max(canvas.width/sourceWidth,canvas.height/sourceHeight),drawWidth=sourceWidth*scale,drawHeight=sourceHeight*scale,left=(canvas.width-drawWidth)/2,top=(canvas.height-drawHeight)/2;
 for(let row=0;row<rows;row++)for(let col=0;col<cols;col++){
  const cell=row*cols+col,cx=(col+.5)*pixel,cy=(row+.5)*pixel,sx=crop.x+Math.min(crop.width-1,Math.floor((cx-left)/drawWidth*crop.width)),sy=crop.y+Math.min(crop.height-1,Math.floor((cy-top)/drawHeight*crop.height)),inside=sx>=0&&sy>=0&&sx<meta.columns&&sy<meta.rows,sourceCell=sy*meta.columns+sx;
  let v=inside?Math.round(frames[offset+sourceCell]*(1-blend)+frames[nextOffset+sourceCell]*blend):0;if(settings.invert&&inside)v=255-v;const g=v/255,seed=cell*31+glyphTick*1337;
  ctx.fillStyle=bg[v];if(glyphTick%8===0&&noise(seed+891)<randomness*.002)ctx.fillStyle=css(hslRgb(palette.bgHue,settings.saturation,5+noise(seed+612)*randomness*3));ctx.fillRect(col*pixel,row*pixel,pixel,pixel);
  if(updateGlyphs){let index=Math.floor(g*(ramp.length-1));const randomColumn=noise(col*7919+179)<randomness,start=Math.floor(noise(col*7919+211)*rows);if(randomColumn){glyphs[cell]=((glyphTick+start)%100)/100*rows+start>row?ramp[index]:''}else{if(noise(seed)<.005*randomness)index=Math.floor(noise(seed+93)*ramp.length);glyphs[cell]=ramp[index]}}
  const visibility=Math.max(0,Math.min(1,(g-threshold)/.045));
  ctx.globalAlpha=visibility;ctx.fillStyle=fg[v];ctx.fillText(glyphs[cell]||' ',col*pixel,(row+1)*pixel);ctx.globalAlpha=1;
 }
 display.drawImage(surface,0,0);
 $('seek').value=position/meta.frames;const divisor=meta.sourceFps*(settings.speed||1);$('clock').textContent=`${time(position/divisor)} / ${time(meta.frames/divisor)} · ${cols}×${rows} символов${settings.speed===0?' · движение остановлено':''}`;
}
function time(v){return Math.floor(v/60)+':'+String(Math.floor(v%60)).padStart(2,'0')}
function tick(now){if(lastTime!==null&&playing){const dt=Math.min((now-lastTime)/1000,.2);position=(position+dt*meta.sourceFps*settings.speed)%meta.frames;glyphTime+=dt*settings.glyphSpeed}lastTime=now;draw();raf=requestAnimationFrame(tick)}
$('play').onclick=()=>{playing=!playing;$('play').textContent=playing?'Пауза':'Продолжить'};
$('seam').onclick=()=>{position=Math.max(0,meta.frames-6*meta.sourceFps*settings.speed);playing=true;lastFrame=-1;$('play').textContent='Пауза';draw()};
$('seek').oninput=()=>{position=Math.min(meta.frames-1,Number($('seek').value)*meta.frames);lastFrame=-1;draw()};
$('accent').oninput=()=>{settings.accent=$('accent').value;sync();makePalette()};document.querySelectorAll('[data-color]').forEach(b=>b.onclick=()=>{settings.accent=b.dataset.color;sync();makePalette()});
$('secondary').oninput=()=>{settings.secondary=$('secondary').value;sync();makePalette()};
$('reset').onclick=()=>{settings={...defaults};glyphKey='';sync();makePalette()};
$('copy').onclick=async()=>{try{await navigator.clipboard.writeText(JSON.stringify(settings,null,2));$('copy').textContent='Скопировано'}catch{$('copy').textContent='Выдели настройки ниже и скопируй'}};
document.addEventListener('visibilitychange',()=>{cancelAnimationFrame(raf);lastTime=null;if(!document.hidden&&frames)raf=requestAnimationFrame(tick)});

const themeId=new URLSearchParams(location.search).get('theme');
if(!/^theme-[a-f0-9]{64}$/.test(themeId||''))throw Error('Unknown theme');
let motionEnabled=true;
function resizePlayer(){canvas.width=Math.min(1600,innerWidth);canvas.height=Math.round(canvas.width*innerHeight/innerWidth);surface.width=canvas.width;surface.height=canvas.height;glyphKey='';lastFrame=-1;if(frames)draw()}
addEventListener('resize',resizePlayer);
addEventListener('message',e=>{
 if(e.source!==parent||e.origin!==location.origin)return;const m=e.data;
 if(m?.type==='motion'){motionEnabled=!!m.enabled;playing=motionEnabled;cancelAnimationFrame(raf);lastTime=null;if(!document.hidden&&frames&&motionEnabled)raf=requestAnimationFrame(tick)}
 if(m?.type==='themeData'&&m.theme===themeId&&!frames){try{
  const manifest=m.manifest;meta=manifest.data;
  if(!(m.frames instanceof ArrayBuffer)||m.frames.byteLength>64*1024*1024||m.frames.byteLength!==meta.uncompressedBytes||meta.columns*meta.rows*meta.frames!==m.frames.byteLength)throw Error('Invalid theme size');
  settings={...defaults,...manifest.settings};sync();makePalette();frames=new Uint8Array(m.frames);motionEnabled=!!m.enabled;resizePlayer();playing=motionEnabled;if(motionEnabled)raf=requestAnimationFrame(tick);parent.postMessage({type:'themeReady',theme:themeId},location.origin);
 }catch(e){parent.postMessage({type:'themeError',theme:themeId,message:String(e)},location.origin)}}
});
document.addEventListener('visibilitychange',()=>{cancelAnimationFrame(raf);lastTime=null;if(!document.hidden&&frames&&motionEnabled)raf=requestAnimationFrame(tick)});
