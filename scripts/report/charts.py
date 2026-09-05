from PIL import Image, ImageDraw, ImageFont
from pathlib import Path
import json
P=Path(__file__).resolve().parents[2]/'docs/evidence/2026-09-05'
S=json.loads((P/'state-after.json').read_text());B=json.loads((P/'state-before.json').read_text());D=json.loads((P/'decision.json').read_text())
import os
FONT=os.environ.get('REPORT_FONT', '/Library/Fonts/Arial Unicode.ttf')
def f(n):return ImageFont.truetype(FONT,n)
BLUE='#316aa7';GRAY='#718096';RED='#bc4145';GREEN='#28805b';INK='#24313f';LIGHT='#e5eaf0'
def base(title,sub='',h=850):
 im=Image.new('RGB',(1600,h),'white');d=ImageDraw.Draw(im);d.text((55,28),title,font=f(38),fill=INK)
 if sub:d.text((55,88),sub,font=f(24),fill=GRAY)
 return im,d
def save(im,name):im.save(P/name)
def text(d,xy,s,size=25,fill=INK):d.text(xy,s,font=f(size),fill=fill)
def arrow(d,x,y,x2,y2):
 d.line((x,y,x2,y2),fill=GRAY,width=4);d.polygon([(x2,y2),(x2-10,y2-15),(x2+10,y2-15)],fill=GRAY)
# Architecture
im,d=base('系统分层架构','输入与展示在 Web，领域计算与持久化在 Rust',810)
rows=[('Web 界面','src/web/index.html','看板、表单、成员负载、决策矩阵与响应式交互'),('HTTP 与 API','src/http.rs  /  src/api.rs','请求解析 → 参数校验 → 副本修改 → 保存成功后提交状态'),('领域模型与算法','model.rs  /  economics.rs  /  insight.rs','成员和任务模型；工时、NPV、派工、投资组合与健康度'),('存储与日期工具','src/store.rs  /  src/date.rs','JSON 临时文件写入与重命名；真实日期校验与工作日换算')]
for i,(name,files,desc) in enumerate(rows):
 y=145+i*157;d.rounded_rectangle((60,y,1540,y+116),radius=10,fill='#f2f6fa',outline='#bccddd',width=2)
 text(d,(85,y+16),name,30);text(d,(410,y+13),files,25,BLUE);text(d,(410,y+59),desc,26)
 if i<3:arrow(d,800,y+120,800,y+150)
save(im,'architecture.png')
# Domain
im,d=base('领域模型与关联','任务负责人是可空的成员编号；删除成员后保留任务并取消负责人',750)
for x,title,lines in [(60,'Member  成员',['id / name：编号与姓名','role：学科角色','weekly_hours：周产能','hourly_cost：小时成本']),(850,'Task  任务',['id / title：编号与标题','skill / priority / status','estimate / due：工时与截止日','value / delay_cost_per_day','assignee：可空的 Member.id'])]:
 d.rounded_rectangle((x,155,x+680,590),radius=10,fill='#f5f7fa',outline='#b5c5d6',width=2);text(d,(x+30,178),title,34,BLUE)
 for j,l in enumerate(lines):text(d,(x+30,253+j*55),l,27)
d.line((742,340,848,340),fill=GRAY,width=4);text(d,(744,280),'1 : N',26)
text(d,(70,640),'Role：前端 / 后端 / 测试 / 产品 / 运维     Status：待办 / 进行中 / 待评审 / 已完成',27)
save(im,'domain.png')
# State distributions, clearly separated units
im,d=base('任务状态分布','默认演示数据；数量与基准工时分别统计，已完成任务不计入负载',750)
for idx,(label,field,maxv) in enumerate([('任务数（个）','count',10),('基准工时（h）','hours',120)]):
 ox=100+idx*790;text(d,(ox,150),label,30)
 vals=[]
 for key in ['todo','doing','review','done']:
  tasks=[t for t in S['tasks'] if t['status']==key];vals.append(len(tasks) if field=='count' else sum(t['estimate'] for t in tasks))
 for j,(n,v) in enumerate(zip(['待办','进行中','待评审','已完成'],vals)):
  x=ox+35+j*152;y=590;h=340*v/maxv;d.rectangle((x,y-h,x+85,y),fill=[BLUE,'#c39138','#7892b0',GREEN][j]);text(d,(x,y-h-40),str(int(v)),28);text(d,(x-8,y+20),n,24)
 d.line((ox,590,ox+655,590),fill=GRAY,width=2)
save(im,'status.png')
# Load comparison
im,d=base('自动派工前后的成员负载','负载率 = 未完成任务有效工时 / 周产能；浅色为派工前，深色为派工后',760)
ox=255;scale=8.2
for ratio,col in [(85,'#b48a32'),(100,RED),(130,GRAY)]:
 x=ox+ratio*scale;d.line((x,160,x,660),fill=col,width=2);text(d,(x-32,125),str(ratio)+'%',22,col)
for i,(a,b) in enumerate(zip(B['workloads'],S['workloads'])):
 y=198+i*94;text(d,(65,y),b['name'],30)
 d.rectangle((ox,y,ox+a['ratio']*100*scale,y+23),fill='#cad8e7')
 d.rectangle((ox,y+28,ox+b['ratio']*100*scale,y+57),fill=RED if b['ratio']>1 else '#b68a30')
 text(d,(ox+b['ratio']*100*scale+12,y+24),f"{b['ratio']*100:.0f}%  {b['assigned']:.1f}/{b['capacity']:.0f}h",24)
text(d,(65,694),'派工后健康度：74 分。触发过载约束，明确提示减少在手任务或重新派工。',27)
save(im,'workloads.png')
# Full coefficients matrix
roles=['前端','后端','测试','产品','运维']; factors=[[1,1.25,1.4,1.7,1.35],[1.25,1,1.4,1.7,1.35],[1.4,1.4,1,1.7,1.5],[1.7,1.7,1.7,1,1.7],[1.35,1.35,1.5,1.7,1]]
im,d=base('跨学科工时系数','模型假设，未经历史数据拟合；不同学科还需另加 2 小时沟通工时',800)
text(d,(70,160),'承接人 ↓ / 任务 →',25)
for j,r in enumerate(roles):text(d,(415+j*217,160),r,30)
for i,r in enumerate(roles):
 y=225+i*92;text(d,(100,y+22),r,30)
 for j,v in enumerate(factors[i]):
  x=330+j*217;d.rectangle((x,y,x+207,y+82),fill='#e7eef6' if v==1 else '#f3f6f9',outline='#c5d0dc',width=1);text(d,(x+64,y+21),f'{v:.2f}',30,BLUE)
text(d,(65,717),'例如：测试承接 9h 前端任务 → 9 × 1.40 + 2 = 14.6h',28)
save(im,'coefficients.png')
# Decision
im,d=base('单任务候选方案净现值','消息推送重复投递修复；同一派工后快照，绿色为推荐，灰色为产能不可行',780)
for i,p in enumerate(D['plans']):
 y=180+i*105;text(d,(60,y),p['name'],30)
 width=max(1,p['npv']/25000*970);d.rectangle((230,y,230+width,y+54),fill=GREEN if p['feasible'] else '#a6b2be')
 text(d,(245+width,y+4),f"¥{p['npv']:,.0f} / {p['load_after']*100:.0f}%",25)
text(d,(60,717),'何言：8h × ¥380/h = ¥3,040；预计 2026-09-11 完成，净现值约 ¥24,097。',26)
save(im,'decision-chart.png')
# Portfolio horizontal makes labels legible
im,d=base('本周投资组合建议','单位工时净现值（元/h）；绿色为纳入，灰色为顺延或需调整',1090)
items=S['economics']['portfolio']['items'];ox=760;scale=.20
for i,p in enumerate(items):
 y=155+i*70;text(d,(50,y+8),p['title'],25);v=p['density'];x=ox+v*scale
 d.rectangle((min(ox,x),y,max(ox+1,x),y+40),fill=GREEN if p['selected'] else '#a6b2be');text(d,(max(ox,x)+15,y+4),f'{v:.0f}',24)
d.line((ox,140,ox,995),fill=GRAY,width=2)
text(d,(50,1020),'使用 132.8h / 134h；这是受约束的贪心建议，不保证全局最优。',28)
save(im,'portfolio.png')
print('7 figures generated')
