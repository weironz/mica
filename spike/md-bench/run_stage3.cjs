// Narrow follow-up: Rust internal timing only, original 3 P0 inputs, 3 rounds.
async (page) => {
 await page.waitForFunction(()=>globalThis.micaMd?.stage3Probe,{timeout:30000});
 const result=await page.evaluate(async()=>{
  const inputs=await(await fetch('stage3-inputs.json',{cache:'no-store'})).json();
  const rounds=[];
  for(let round=1;round<=3;round++) {
   const timer=JSON.parse(micaMd.stage3TimerBaseline(100000));
   const cases={};
   for(const [key,src] of Object.entries(inputs)) {
    const n={p0_87:8192,p0_10k:128,p0_100k:16}[key];
    for(let warm=0;warm<3;warm++) micaMd.stage3Probe(src,n);
    const samples=[];
    for(let i=0;i<15;i++) samples.push(JSON.parse(micaMd.stage3Probe(src,n)));
    const median=field=>samples.map(s=>s[field]).sort((a,b)=>a-b)[7];
    cases[key]={characters:src.length,iterations:n,projectionMedianMs:median('projectionMs'),
      encodeMedianMs:median('encodeMs'),samples};
   }
   rounds.push({round,timer,cases});
  }
  const artifacts={};
  for(const file of ['mica_md.js','mica_md_bg.wasm']) {
   const b=await(await fetch('mica_md/'+file,{cache:'no-store'})).arrayBuffer();
   artifacts[file]={bytes:b.byteLength,sha256:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',b)),x=>x.toString(16).padStart(2,'0')).join('')};
  }
  return {rounds,artifacts,browser:navigator.userAgent,crossOriginIsolated};
 });
 return result;
}
