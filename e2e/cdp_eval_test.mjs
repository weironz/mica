import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {launchBrowser} from './browser.mjs';
import {evaluatePage, waitForPage} from './cdp_eval.mjs';
const server=createServer((req,res)=>{
  res.setHeader('Content-Security-Policy-Report-Only',"script-src 'self'; object-src 'none'");
  if(req.url==='/good.js'||req.url==='/bad.js'){
    res.setHeader('Content-Type','application/javascript');
    res.end(req.url==='/good.js'?'window.marker=42;':"window.marker=new Function('return 42')();");
  }else{res.setHeader('Content-Type','text/html');res.end(`<script src="${req.url==='/bad'?'/bad.js':'/good.js'}"></script>`);}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
let browser;
try{
 browser=await launchBrowser();const page=await browser.newPage();const reports=[];
 page.on('console',m=>{if(m.type()==='error')reports.push(m.text())});
 await page.goto(`http://127.0.0.1:${server.address().port}/`);
 await waitForPage(page,()=>window.marker===42,5000);
 assert.equal(await evaluatePage(page,()=>window.marker),42);
 assert.equal(reports.length,0,'the reader itself must not create CSP reports');
 await page.goto(`http://127.0.0.1:${server.address().port}/bad`);
 await waitForPage(page,()=>window.marker===42,5000);
 assert(reports.some(t=>/unsafe-eval/.test(t)),'actual page evaluation violations must remain observable');
 console.log('CSP probe transport: clean read/poll; actual unsafe page evaluation still reported');
}finally{if(browser)await browser.close();await new Promise(r=>server.close(r));}
