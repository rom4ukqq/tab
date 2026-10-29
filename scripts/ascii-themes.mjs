// Offline ASCII theme preparation. Not imported by the desktop application.
// Character ramp and grayscale mapping adapted from collidingScopes/ascii:
// af6b92f7c48e2b46f7f29d18ba8409a0f93900ff, ASCII.js (MIT, Alan Ang 2024).
// See third_party/notices/ascii-LICENSE.txt. All procedural artwork is original.
// Run: node scripts/ascii-themes.mjs > themes.json (returns filename -> SVG).
const ramp = "____``..--^^~~<>??123456789%%&&@@".replaceAll('_', ' ');
const clamp = v => Math.min(1, Math.max(0, v));
const bell = (v, width) => Math.exp(-((v / width) ** 2));
function jelly(x, y, cx, cy, scale) {
  const u = (x - cx) / scale, v = (y - cy) / scale;
  const radius = Math.sqrt(u*u + (v*1.45)**2);
  let light = v < .08 && v > -.8 ? bell(radius - .65, .075) * .9 : 0;
  if (v < .05 && v > -.48 && Math.abs(u) < .65) {
    light = Math.max(light, clamp(1-radius/.75)*.45 + bell(v-.015,.03)*.55);
  }
  if (v > .03 && v < 1.3) {
    for (let n=-3; n<=3; n++) {
      const curve = n*.13 + Math.sin(v*6+n*.9)*(.04+v*.12);
      light = Math.max(light, bell(u-curve,.016)*(1-v/1.5)*.85);
    }
  }
  return light;
}
function field(theme, x, y) {
  if (theme === 'jellyfish') return Math.max(jelly(x,y,.24,.27,.37), jelly(x,y,.79,.38,.46));
  if (theme === 'orbit') {
    const u=(x-.72)*1.6, v=(y-.49);
    const r=Math.sqrt(u*u+v*v);
    const sphere = r<.29 ? (.22+.53*Math.sqrt(1-(r/.29)**2))*(.6+.4*Math.cos(u*35)*Math.cos(v*35)) : 0;
    const ring=Math.sqrt((u*.87+v*.5)**2 + ((v*.87-u*.5)*3.8)**2);
    return Math.max(sphere,bell(r-.29,.009)*.9,bell(ring-.49,.014)*.8);
  }
  let light=0;
  for(let n=0;n<7;n++){
    const line=.32+n*.055 + Math.sin(x*7+n*.32)*(.09+x*.1);
    light=Math.max(light,bell(y-line,.012)*(.55+n*.06));
  }
  return light;
}
function makeSvg(theme) {
  const groups=Array.from({length:6},()=>[]);
  for(let row=0;row<60;row++){
    const cells=Array.from({length:6},()=>({x:[],text:''}));
    for(let col=0;col<120;col++){
      const value=clamp(field(theme,col/119,row/59));
      // Same luminance-to-character mapping as the offline reference generator.
      const gray=255*(.299*value+.587*value+.114*value);
      const char=ramp[Math.floor(gray/255*(ramp.length-1))];
      if(char===' ' || value<.13) continue;
      const level=Math.min(5,Math.floor(value*6));
      cells[level].x.push(col*12);
      cells[level].text+=char;
    }
    cells.forEach((cell,level)=>{
      if(cell.x.length) groups[level].push('<text x="'+cell.x.join(' ')+'" y="'+(row*15+12)+'">'+cell.text.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;')+'</text>');
    });
  }
  return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1440 900"><title>'+theme+' ASCII artwork</title><g fill="white" font-family="Consolas,Courier New,monospace" font-size="12">'+groups.map((rows,i)=>'<g opacity="'+(.25+i*.15).toFixed(2)+'">'+rows.join('')+'</g>').join('')+'</g></svg>\n';
}
export function buildThemes() {
  return Object.fromEntries(['jellyfish','orbit','waves'].map(name=>[name+'.svg',makeSvg(name)]));
}
console.log(JSON.stringify(buildThemes()));
