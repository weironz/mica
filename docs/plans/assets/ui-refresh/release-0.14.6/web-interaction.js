async page => {
  await page.setViewportSize({width:1440,height:1000});
  await page.addInitScript(() => {
    window.__micaReleaseSmokeErrors=[];
    window.addEventListener('error',e=>window.__micaReleaseSmokeErrors.push(String(e.message)));
    window.addEventListener('unhandledrejection',e=>window.__micaReleaseSmokeErrors.push(String(e.reason)));
    localStorage.setItem('mica.uiLanguage','zh');
    localStorage.setItem('mica.themeMode','dark');
  });
  await page.goto('https://mica.cloudcele.com/',{waitUntil:'domcontentloaded',timeout:60000});
  await page.locator('flt-semantics-placeholder').waitFor({state:'attached',timeout:60000});
  await page.locator('flt-semantics-placeholder').evaluate(e=>e.click());
  await page.getByText('CRDT SYNC · OFFLINE-FIRST · v0.14.6',{exact:true}).waitFor();
  await page.getByRole('button',{name:'注册',exact:true}).click();
  await page.getByRole('textbox',{name:'显示名称',exact:true}).waitFor({state:'visible'});
  await page.getByRole('button',{name:'登录',exact:true}).click();
  await page.getByRole('textbox',{name:'显示名称',exact:true}).waitFor({state:'hidden'});
  await page.getByRole('button',{name:'忘记密码？',exact:true}).waitFor({state:'visible'});
  const errors=await page.evaluate(()=>window.__micaReleaseSmokeErrors);
  if(errors.length)throw new Error(JSON.stringify(errors));
  return {version:'0.14.6',theme:'dark',registerTabOpened:true,loginTabReturned:true,submittedForms:false,errors};
}
