// Loopback object store and configured HTTP proxy for the isolated API tests.
import { createHash } from 'node:crypto';
const objects=new Map();
const events=[];
let releasePut;
let heldKey;
let holdPut=false;
Bun.serve({hostname:'127.0.0.1',port:Number(process.env.FAKE_S3_PORT),async fetch(req){
  const url=new URL(req.url);
  if(url.pathname==='/fixture-image.png') return new Response(Buffer.alloc(600,Number(url.searchParams.get('byte')??3)),{headers:{'content-type':'image/png'}});
  if(url.pathname==='/_hold-put') { holdPut=true; heldKey=null; return Response.json({ok:true}); }
  if(url.pathname==='/_held') return Response.json({key:heldKey});
  if(url.pathname==='/_release-put') { releasePut?.(); holdPut=false; return Response.json({ok:true}); }
  if(url.pathname==='/_events') return Response.json(events);
  if(url.pathname==='/mica'||url.pathname==='/mica/') return new Response(null,{status:200});
  const key=decodeURIComponent(url.pathname.replace(/^\/mica\//,''));
  if(req.method==='PUT') {
    if(holdPut) { heldKey=key; await new Promise(resolve=>{ releasePut=resolve; }); }
    const bytes=Buffer.from(await req.arrayBuffer());
    if(req.headers.get('if-none-match')==='*'&&objects.has(key)) return new Response(null,{status:412});
    const checksum=createHash('sha256').update(bytes).digest('base64');
    if(req.headers.get('x-amz-checksum-sha256')!==checksum) return new Response('checksum mismatch',{status:400});
    objects.set(key,{bytes,checksum});events.push({method:'PUT',key});
    return new Response(null,{status:200});
  }
  if(req.method==='DELETE') {objects.delete(key);events.push({method:'DELETE',key});return new Response(null,{status:204});}
  const object=objects.get(key);
  if(!object) return new Response(null,{status:404});
  events.push({method:req.method,key});
  return new Response(req.method==='HEAD'?null:object.bytes,{headers:{'content-length':String(object.bytes.length),'content-type':'image/png','x-amz-checksum-sha256':object.checksum,'last-modified':'Mon, 01 Jan 2024 00:00:00 GMT'}});
}});
