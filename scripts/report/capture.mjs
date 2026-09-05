import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_PATH || 'playwright');
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const out=path.join(root,'docs/evidence/2026-09-05');
const tmp=fs.mkdtempSync('/private/tmp/moonlight-report-');
const server=spawn(path.join(root,'target/release/moonlight'),['--port','18089','--data',path.join(tmp,'demo.json')]);
let log='';server.stdout.on('data',b=>log+=b);server.stderr.on('data',b=>log+=b);
const save=(n,v)=>fs.writeFileSync(path.join(out,n),typeof v==='string'?v:JSON.stringify(v,null,2));
let browser;
try{
 let ready=false;for(let i=0;i<50;i++){try{const r=await fetch('http://127.0.0.1:18089/api/state');if(r.ok){ready=true;break;}}catch{}await new Promise(r=>setTimeout(r,100));}if(!ready)throw Error('Server failed: '+log);
 browser=await chromium.launch({...(process.env.CHROME_EXECUTABLE ? {executablePath:process.env.CHROME_EXECUTABLE} : {}),headless:true});
 const page=await browser.newPage({viewport:{width:1920,height:1080},deviceScaleFactor:1.5});
 const failures=[];page.on('pageerror',e=>failures.push(e.message));
 await page.goto('http://127.0.0.1:18089');await page.locator('.card').first().waitFor();
 save('state-before.json',await (await fetch('http://127.0.0.1:18089/api/state')).json());
 await page.screenshot({path:path.join(out,'ui-before.png')});
 await page.getByRole('button',{name:'按模型推荐派工',exact:true}).click();
 await page.locator('#toast').filter({hasText:/已派工/}).waitFor();
 const after=await (await fetch('http://127.0.0.1:18089/api/state')).json();save('state-after.json',after);
 await page.locator('#toast').evaluate(el=>el.classList.remove('show'));
 await page.screenshot({path:path.join(out,'ui-main.png')});
 await page.locator('[data-decide="4"]').click();await page.locator('#dmask.show').waitFor();
 save('decision.json',await (await fetch('http://127.0.0.1:18089/api/tasks/4/decision')).json());
 await page.locator('#dmask .modal').screenshot({path:path.join(out,'ui-decision.png')});
 await page.getByRole('button',{name:'关闭',exact:true}).click();
 await page.getByRole('button',{name:'新建任务',exact:true}).click();await page.locator('#mask.show').waitFor();
 await page.locator('#mask .modal').screenshot({path:path.join(out,'ui-form.png')});
 await page.getByRole('button',{name:'取消',exact:true}).click();
 // Exercise the actual drag handler, then restore the isolated demo state.
 await page.locator('.card[data-id="4"]').dragTo(page.locator('.col[data-status="doing"]'), {sourcePosition:{x:20,y:20},targetPosition:{x:30,y:20}});
 await page.waitForFunction(()=>document.querySelector('.col[data-status="doing"] .card[data-id="4"]'));
 let check=await (await fetch('http://127.0.0.1:18089/api/state')).json();if(check.tasks.find(t=>t.id===4).status!=='doing')throw Error('Drag did not persist');
 await fetch('http://127.0.0.1:18089/api/tasks/4',{method:'PATCH',headers:{'Content-Type':'application/json'},body:JSON.stringify({status:'todo'})});
 const apiTests=[];
 for(const [name,body] of [
  ['非法日期',{title:'日期验收',skill:'backend',priority:'p1',estimate:4,value:1000,due:'2026-02-31'}],
  ['空标题',{title:'  ',skill:'backend',priority:'p1',estimate:4,value:1000,due:'2026-09-30'}],
  ['负工时',{title:'工时验收',skill:'backend',priority:'p1',estimate:-1,value:1000,due:'2026-09-30'}],
  ['不存在负责人',{title:'负责人验收',skill:'backend',priority:'p1',estimate:4,value:1000,due:'2026-09-30',assignee:9999}]
 ]){const r=await fetch('http://127.0.0.1:18089/api/tasks',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});apiTests.push({name,method:'POST',path:'/api/tasks',request:body,status:r.status,response:await r.json()});if(r.status!==400)throw Error(name+' expected 400');}
 check=await (await fetch('http://127.0.0.1:18089/api/state')).json();if(check.tasks.length!==after.tasks.length)throw Error('Invalid requests changed state');
 save('api-tests.json',apiTests);
 const mobile=await browser.newPage({viewport:{width:390,height:844},deviceScaleFactor:2});await mobile.goto('http://127.0.0.1:18089');await mobile.locator('.card').first().waitFor();
 await mobile.locator('.card[data-id="4"]').scrollIntoViewIfNeeded();
 if(await mobile.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw Error('Mobile overflow');
 await mobile.locator('[data-status-select="4"]').selectOption('doing');
 await mobile.waitForFunction(()=>document.querySelector('.col[data-status="doing"] .card[data-id="4"]'));
 await mobile.locator('.col[data-status="doing"]').scrollIntoViewIfNeeded();
 await mobile.screenshot({path:path.join(out,'ui-mobile.png')});
 await fetch('http://127.0.0.1:18089/api/tasks/4',{method:'PATCH',headers:{'Content-Type':'application/json'},body:JSON.stringify({status:'todo'})});
 save('ui-tests.json',{browser:await browser.version(),viewport:'1920x1080; mobile 390x844',autoAssign:true,dragPersisted:true,mobileStatusPersisted:true,mobileNoHorizontalOverflow:true,invalidRequestsDidNotCreateTasks:true,pageErrors:failures});
 if(failures.length)throw Error(failures.join('\n'));
 // Faithful, readable source/log excerpts, explicitly labelled as excerpts rather than an IDE.
 const esc=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;');
 const view=async(name,title,body)=>{const p=await browser.newPage({viewport:{width:1440,height:1000},deviceScaleFactor:1.5});await p.setContent(`<html lang="zh-CN"><meta charset="utf-8"><style>body{margin:0;background:#fff;color:#20252b;font:20px/1.65 -apple-system,"PingFang SC",sans-serif}h1{font-size:25px;font-weight:600;margin:0 0 16px}main{padding:30px 36px}pre{font:18px/1.65 Menlo,"PingFang SC",monospace;white-space:pre-wrap;overflow-wrap:anywhere;margin:0}small{color:#66717d}</style><main><h1>${esc(title)}</h1><pre>${esc(body)}</pre></main></html>`);await p.locator('main').screenshot({path:path.join(out,name)});await p.close();};
 for(const [name,file,start,end] of [['code-economics.png','src/economics.rs','pub fn discipline_factor','/// 延期日损失'],['code-web.png','src/web/index.html','function bindBoard(){','/* 决策矩阵'],['code-tests.png','src/insight.rs','    fn 全员均匀过载也不能判为健康()', '    #[test]\n    fn 自动派工应保留'],['code-readme.png','README.md','## 计量口径','## 运行']]){
  const content=fs.readFileSync(path.join(root,file),'utf8');let a=content.indexOf(start),b=content.indexOf(end,a+start.length);if(a<0||b<0)throw Error('Excerpt marker '+file);const first=content.slice(0,a).split('\n').length;const lines=content.slice(a,b).trimEnd().split('\n');await view(name,`${file}  源码节选`,lines.map((l,i)=>`${String(first+i).padStart(3)}  ${l}`).join('\n'));
 }
 const checks=JSON.parse(fs.readFileSync(path.join(out,'checks.json'))).checks;
 await view('checks.png','构建与测试执行记录',checks.map(c=>'$ '+c.command+'\n'+(c.command.includes('test')?c.output.split('\n').filter(x=>/running |test result:/.test(x)).join('\n'):c.output.trim())+'\n退出码 '+c.exit_code).join('\n\n'));
 await view('api-tests.png','非法输入验收  真实请求与返回',apiTests.map(t=>t.name+'\n'+t.method+' '+t.path+'\n'+JSON.stringify(t.request)+'\nHTTP '+t.status+'\n'+JSON.stringify(t.response)).join('\n\n'));
 save('demo-after.json',fs.readFileSync(path.join(tmp,'demo.json'),'utf8'));save('server.txt',log);
 console.log(JSON.stringify({after:after.health,summary:after.economics.total_npv,browser:await browser.version(),out}));
}finally{if(browser)await browser.close();server.kill();}
