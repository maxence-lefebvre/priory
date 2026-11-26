-- The state scripts/bench/answer-latency.py starts from: 1,500 tasks (60 open), 30,000 inbox rows of about 1.3 KB, 400 open outside asks and 1,500 answered ones.
PRAGMA foreign_keys=off;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1500)
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch)
SELECT i,'ENG-'||i,'implement',CASE WHEN i>1440 THEN 'approve-pr' ELSE 'done' END,CASE WHEN i>1440 THEN 'asleep' ELSE 'done' END,'later','payments','linear','seed-'||i,'acme/shop','Synthetic task number '||i||' that changes one thing',
strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d minutes',(1500-i)*20)),strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d minutes',(1500-i)*18)),strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d minutes',(1500-i)*18)),'claude-sonnet-5-5','eng-'||i FROM n;
INSERT INTO pull_requests (task,resource,branch,base,state) SELECT id,'github:acme/shop#'||id,branch,'main',CASE WHEN id>1440 THEN 'open' ELSE 'merged' END FROM tasks;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<30000)
INSERT INTO inbox (id,type,subject,time,data,links,task,outcome)
SELECT 'i'||i,CASE i%5 WHEN 0 THEN 'github.pull_request.closed' WHEN 1 THEN 'github.issue_comment.created' ELSE 'github.check_run.completed' END,'acme/shop#'||(1+i%1500),
strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d minutes',(30000-i))),
json_object('repo','acme/shop','pull_request',1+i%1500,'head_sha',printf('%040x',i),'head_branch','eng-'||(1+i%1500),'base_branch','main','name','check '||(i%30),'conclusion','success','state','open','title','t','url','https://github.com/acme/shop/pull/1','pad',printf('%.*c',1000,'x')),
'[]',CASE WHEN i%3=0 THEN 1+i%1500 END,CASE WHEN i%3=0 THEN 'routed' ELSE 'ignored' END FROM n;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<400)
INSERT INTO asks (id,name,kind,question,options,recommendation,title,text) SELECT i,'bench-'||i,'outside','Pick one for '||i,'["yes","no"]','yes','Ask '||i,printf('%.*c',1500,'t') FROM n;
WITH RECURSIVE n(i) AS (SELECT 401 UNION ALL SELECT i+1 FROM n WHERE i<1900)
INSERT INTO asks (id,task,step,kind,question,options,answer,answered,answered_by,title,text) SELECT i,i-400,'approve-pr','step','Open?','["open","stop"]','open',strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d minutes',i*18)),'serve','Open',printf('%.*c',1500,'t') FROM n;
