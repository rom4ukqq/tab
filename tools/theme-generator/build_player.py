"""Generate the packaged background player from the same renderer as the editor."""
from pathlib import Path
import re
import shutil

root = Path(__file__).resolve().parent
destination = root.parents[1] / 'apps/desktop/public/themes'
destination.mkdir(parents=True, exist_ok=True)
html = (root / 'index.html').read_text(encoding='utf-8')
script = re.search(r'<script>\s*([\s\S]*?)</script>', html)[1]
script = script[:script.index('// Portable ZIP')]
script = script.replace("localStorage.getItem('ascii-preview-settings-v3')", "null")
script += r'''
const themeId=new URLSearchParams(location.search).get('theme');
if(!['saveli','pr0n'].includes(themeId))throw Error('Unknown theme');
let motionEnabled=true;
function resizePlayer(){canvas.width=Math.min(1600,innerWidth);canvas.height=Math.round(canvas.width*innerHeight/innerWidth);surface.width=canvas.width;surface.height=canvas.height;glyphKey='';lastFrame=-1;if(frames)draw()}
addEventListener('resize',resizePlayer);
addEventListener('message',e=>{if(e.source!==parent)return;const m=e.data;if(m?.type==='motion'){motionEnabled=!!m.enabled;playing=motionEnabled;cancelAnimationFrame(raf);lastTime=null;if(!document.hidden&&frames&&motionEnabled)raf=requestAnimationFrame(tick)}});
document.addEventListener('visibilitychange',()=>{cancelAnimationFrame(raf);lastTime=null;if(!document.hidden&&frames&&motionEnabled)raf=requestAnimationFrame(tick)});
(async()=>{const response=await fetch(`./${themeId}/theme.json`);if(!response.ok)throw Error('Theme metadata unavailable');const manifest=await response.json();meta=manifest.data;settings={...defaults,...manifest.settings};sync();makePalette();const result=await fetch(`./${themeId}/frames.bin.gz`);if(!result.ok)throw Error('Theme data unavailable');frames=new Uint8Array(await new Response(result.body.pipeThrough(new DecompressionStream('gzip'))).arrayBuffer());if(frames.length!==meta.uncompressedBytes)throw Error('Invalid theme size');resizePlayer();playing=motionEnabled;if(motionEnabled)raf=requestAnimationFrame(tick);parent.postMessage({type:'themeReady',theme:themeId},location.origin)})().catch(e=>parent.postMessage({type:'themeError',theme:themeId,message:String(e)},location.origin));
'''
# Static editor controls satisfy renderer bindings but are never shown to app users.
html = re.sub(r'<script src="theme-data.js"></script>\s*<script>[\s\S]*?</script>', '<script src="player.js"></script>', html)
html = html.replace('</style>', 'header,aside,.controls,.hint,#seek{display:none!important}html,body,main,.stage{margin:0!important;padding:0!important;width:100%;height:100%;display:block!important;overflow:hidden;background:#080b10}#art{width:100%;height:100%;border:0;border-radius:0}</style>')
(destination / 'player.html').write_text(html, encoding='utf-8')
(destination / 'player.js').write_text(script, encoding='utf-8')
shutil.copyfile(root / 'LICENSE-ascii.txt', destination / 'LICENSE-ascii.txt')
