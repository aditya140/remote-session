#!/usr/bin/env python3
"""Browser contract checks with intercepted HTTP/WSS fixtures, not deployment proof."""
import argparse
import base64
import json
from pathlib import Path
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright

ROOT=Path(__file__).resolve().parents[1]
USER='u_550e8400-e29b-41d4-a716-446655440001'
MACHINE='m_550e8400-e29b-41d4-a716-446655440001'
S1='s_550e8400-e29b-41d4-a716-446655440001'
S2='s_550e8400-e29b-41d4-a716-446655440002'
BASE='http://127.0.0.1:8877'

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--chromium',default='/usr/bin/chromium');args=parser.parse_args()
 with sync_playwright() as p:
  browser=p.chromium.launch(executable_path=args.chromium,args=['--no-sandbox'],headless=True)
  context=browser.new_context(viewport={'width':1440,'height':1100})
  state={'admin':True,'ended':False,'lost_reply':False,'stops':0,'history_reads':0,'snapshot_reads':0}
  def snapshot():
   return {'observed_at_ms':1793491200000,'coverage':{'complete':True,'users':1,'machines':1,'sessions':2,'limit':500,'freshness_ms':45000},'users':[{'id':USER,'display_name':'<img src=x onerror="window.pwned=1">'}],'machines':[{'id':MACHINE,'owner':USER,'name':'dev-host','os':'Linux','device_class':'server','connected':True,'last_seen_ms':1793491200000,'last_connected_ms':1793481200000,'agents':{}}],'sessions':[{'id':sid,'machine':MACHINE,'owner':USER,'name':'Agent '+str(index),'kind':'agent','harness':'Codex','harness_version':'0.159.2','requested_model':'configured-model','model':None,'model_source':'launch_configuration','state':'exited' if state['ended'] and sid==S1 else 'running','task_state':'unknown','created_at_ms':1793491200000,'observed_at_ms':1793491200000,'terminate':True,'origin':'managed'} for index,sid in enumerate([S1,S2],1)]}
  def route(r):
   path=urlsplit(r.request.url).path
   if path.startswith('/console/assets/'):
    file=ROOT/'assets'/path.split('/')[-1]
    r.fulfill(status=200,body=file.read_text(),content_type='text/css' if file.suffix=='.css' else 'application/javascript');return
   if path in ['/console/','/console/index.html']:
    r.fulfill(status=200,body=(ROOT/'index.html').read_text(),content_type='text/html');return
   if path=='/console/auth/session':data={'authenticated':True,'user':USER,'platform_admin':state['admin'],'csrf_token':'fixture-csrf','expires_at':2000000000}
   elif path=='/v1/me':data={'user':USER,'platform_admin':state['admin'],'expires_at':2000000000}
   elif path.endswith('/overview'):
    state['snapshot_reads']+=1
    data=snapshot()
   elif path=='/v1/admin/health':data={'observed_at_ms':1793491200000,'scope':'accepting_gateway_process','status':'degraded','components':{'database':{'status':'ready'},'routing':{'configured':True,'status':'observed_healthy'},'recording':{'configured':True,'status':'configured','commit_durability':'not_probed'}},'connections':{'clients':2,'hosts':1,'pending':0,'cross_node_messages':0}}
   elif path=='/v1/admin/telemetry/series':data={'available':True,'items':[{'start_ms':1793491200000,'end_ms':1793491500000,'value':2}],'start_ms':1793491200000,'end_ms':1793491500000,'complete':True,'unit':'events'}
   elif path=='/v1/admin/activity-signals':data={'items':[{'id':'signal_fixture','title':'Device enrollment burst','severity':'review','subject':USER,'explanation':'Explicit threshold; not a finding of misuse.','evidence':{'window_start_ms':1793491200000,'window_end_ms':1793491500000,'threshold':3,'distinct_resources':3,'complete':True,'event_ids':['1','2','3']}}],'baseline':{'status':'insufficient_baseline'},'evaluation':'on_demand_metadata_rules'}
   elif path=='/v1/admin/telemetry/runs':data={'available':False,'reason':'scheduled_rule_evaluation_not_configured','items':[]}
   elif path.endswith('/history/streams'):
    assert r.request.headers.get('x-csrf-token')=='fixture-csrf'
    data={'streams':[{'stream_epoch':'epoch_fixture','stream':'pty'}],'next':None}
   elif path.endswith('/history'):
    assert r.request.headers.get('x-csrf-token')=='fixture-csrf'
    state['history_reads']+=1
    payload='<script>window.pwned=2</script>\x1b]52;c;clipboard\x07'
    data={'session':S1,'machine':MACHINE,'stream_epoch':'epoch_fixture','items':[{'kind':'content','id':'chunk_fixture','from':'1','to':'1','redaction_saturated':False,'records':[{'seq':'1','content':{'type':'output','data':base64.b64encode(payload.encode()).decode()}}]},{'kind':'gap','gap':{'reason':'explicit missing output'}}],'next':None}
   elif path=='/console/auth/logout':data={}
   else:r.fulfill(status=404,body='{}',content_type='application/json');return
   r.fulfill(status=200,body=json.dumps(data),content_type='application/json')
  context.route(BASE+'/**',route)
  def socket(ws):
   def received(raw):
    message=json.loads(raw);kind=message['type']
    def send(kind,body):ws.send(json.dumps({'v':1,'id':'msg_550e8400-e29b-41d4-a716-446655440009','re':message['id'],'machine':message.get('machine'),'session':message.get('session'),'type':kind,'body':body}))
    if kind=='hello':
     send('welcome',{'version':{'major':1,'minor':0},'capabilities':['routing-v1','machine-pages-v1','auth-refresh-v1','session-control-v1','session-pages-v1']})
     send('connection.ready',{'client':'client_fixture','host_generation':None,'activated':False})
    elif kind=='session.get':send('session.found',{'id':message['session'],'machine':MACHINE,'owner':USER,'capabilities':{'terminate':True},'origin':'managed','state':'exited' if state['ended'] and message['session']==S1 else 'running'})
    elif kind=='session.host.query':send('session.host',{'generation':'9007199254740993'})
    elif kind=='session.terminate':
     state['stops']+=1
     assert message['body']['host_generation']=='9007199254740993'
     assert message['body']['confirmed'] is True
     if state['lost_reply']:ws.close(code=1011,reason='Lost acknowledgment')
     else:
      assert message['session']==S1;state['ended']=True
      send('session.action.result',{'action':'terminate','outcome':'applied'})
   ws.on_message(received)
  context.route_web_socket('ws://127.0.0.1:8877/v1/connect/client',socket)
  page=context.new_page();errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
  page.goto(BASE+'/console/')
  page.locator(f'[data-session-row="{S1}"]').wait_for()
  assert page.locator('#content').inner_text().find('Configured: configured-model')>=0
  assert page.locator('#content').inner_text().find('Not reported')>=0
  assert page.locator('img').count()==0
  page.locator('button[data-view="admin"]').click();page.locator('.av-person').wait_for()
  assert page.evaluate('window.pwned') is None
  page.locator('#nav button[data-page="Telemetry & signals"]').click();page.locator('.time-series').wait_for()
  assert '2' in page.locator('.time-series').inner_text()
  assert 'scheduled_rule_evaluation_not_configured' not in page.locator('#content').inner_text()
  assert 'Scheduled runs: Not configured' in page.locator('#content').inner_text()
  page.get_by_role('button',name='Inspect evidence').click()
  assert 'Review threshold: 3' in page.locator('#evidence-body').inner_text()
  assert 'Audit event references: 1, 2, 3' in page.locator('#evidence-body').inner_text()
  page.locator('#evidence-close').click()
  page.locator('#nav [data-page="System health"]').click();page.get_by_role('heading',name='The accepting gateway, observed.').wait_for()
  assert 'accepting_gateway_process' in page.locator('#content').inner_text()
  assert 'not_probed' in page.locator('#content').inner_text()
  page.locator('button[data-view="workspace"]').click();page.locator(f'[data-session-row="{S1}"]').wait_for()
  page.locator(f'[data-stop="{S1}"]').click();page.locator('#stop-confirmation').fill('WRONG')
  assert not page.locator('#stop-form').evaluate('(e)=>e.checkValidity()')
  page.locator('#stop-cancel').click();assert state['stops']==0
  page.locator(f'[data-stop="{S1}"]').click();page.locator('#stop-confirmation').fill('STOP');page.locator('#stop-submit').click()
  page.locator(f'[data-session-row="{S1}"]').filter(has_text='Exited').wait_for()
  assert state['stops']==1
  assert 'Running' in page.locator(f'[data-session-row="{S2}"]').inner_text()
  state['lost_reply']=True
  page.locator(f'[data-stop="{S2}"]').click();page.locator('#stop-confirmation').fill('STOP');page.locator('#stop-submit').click()
  page.locator(f'[data-session-row="{S2}"]').filter(has_text='Stop unknown').wait_for()
  page.locator(f'[data-reconcile="{S2}"]').click();page.wait_for_timeout(100)
  assert state['stops']==2
  page.reload();page.locator(f'[data-session-row="{S2}"]').filter(has_text='Stop unknown').wait_for();assert state['stops']==2
  page.locator(f'[data-session-row="{S1}"] [data-history]').click();page.get_by_role('button',name='Read pty stream').click();page.locator('[data-history-record]').wait_for()
  assert '<script>window.pwned=2</script>' in page.locator('[data-history-record]').inner_text()
  assert page.evaluate('window.pwned') is None
  assert 'explicit missing output' in page.locator('#content').inner_text()
  assert state['history_reads']==1
  for width in [390,768,1440]:
   page.set_viewport_size({'width':width,'height':1000});page.locator('#nav [data-page="My devices"]').click();page.locator(f'[data-session-row="{S1}"]').wait_for()
   assert page.evaluate('document.body.scrollWidth<=innerWidth'),width
  draft_page=context.new_page()
  draft_page.clock.install()
  draft_page.goto(BASE+'/console/?view=admin')
  draft_page.locator('.av-person').click()
  draft_page.locator(f'[data-session-row="{S1}"] [data-history]').click()
  draft_page.locator('#history-reason').fill('Investigating an unfinished draft')
  draft_page.locator('#history-case').fill('CASE-DRAFT-1')
  snapshots_before=state['snapshot_reads']
  draft_page.clock.fast_forward(11000)
  draft_page.clock.run_for(100)
  assert draft_page.locator('#history-reason').input_value()=='Investigating an unfinished draft'
  assert draft_page.locator('#history-case').input_value()=='CASE-DRAFT-1'
  assert state['snapshot_reads']==snapshots_before
  draft_page.close()
  assert not errors,errors
  state['admin']=False;page.reload();page.locator(f'[data-session-row="{S1}"]').wait_for();assert page.locator('[data-view="admin"]').count()==0
  browser.close()
 print(json.dumps({'status':'passed','scope':'intercepted browser contracts','checks':['verified role navigation','configured versus effective model','XSS-safe inventory and recording text','exact termination target','64-bit generation preserved','cancel no mutation','unknown stop no resend across reload','other session survives','history gaps','desktop tablet mobile','history reason draft survives auto-poll timer'],'native_deployment':'not established by fixtures'}))

if __name__=='__main__':main()
