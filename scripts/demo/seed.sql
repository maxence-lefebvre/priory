INSERT INTO bands (name, rank) VALUES ('blocker',0),('now',1),('today',2),('later',3) ON CONFLICT DO NOTHING;
-- tasks
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch,reason) VALUES
(1,'ENG-212','implement','approve-pr','asleep','now','payments','linear','seed-1','acme/shop','Retry failed invoice emails with backoff',strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-40 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-40 minutes'),'claude-sonnet-5-5','eng-212-retry-invoice-emails',NULL),
(2,'ENG-219','implement','approve-pr','asleep','today','payments','linear','seed-2','acme/shop','Show the VAT number on the invoice PDF',strftime('%Y-%m-%dT%H:%M:%SZ','now','-7 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'claude-sonnet-5-5','eng-219-vat-on-pdf',NULL),
(3,'ENG-224','implement','implement','blocked','now','search','linear','seed-3','acme/shop','Index product tags in the search service',strftime('%Y-%m-%dT%H:%M:%SZ','now','-9 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'claude-opus-5-5','eng-224-index-tags','The tests need the staging search cluster, which refused the connection'),
(4,'ENG-231','implement','implement','ready','today','search','linear','seed-4','acme/shop','Paginate the order history endpoint',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-12 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-12 minutes'),'claude-sonnet-5-5','eng-231-paginate-orders',NULL),
(5,'ENG-201','implement','done','done','today','payments','linear','seed-5','acme/shop','Add the invoices table',strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-26 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-26 hours'),'claude-sonnet-5-5','eng-201-invoices-table',NULL),
(6,'ENG-238','plan-batch','done','done','later','payments','cli','seed-6','acme/shop','Plan the invoice exports',strftime('%Y-%m-%dT%H:%M:%SZ','now','-6 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),'claude-opus-5-5',NULL,NULL),
(7,'ENG-239','implement','implement','held','later','payments','batch','seed-7','acme/shop','Export invoices as CSV',strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),'claude-sonnet-5-5','eng-239-csv-export','waits for ENG-201 to merge'),
(8,'ENG-240','implement','implement','ready','later','payments','batch','seed-8','acme/shop','Export invoices as PDF',strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),'claude-sonnet-5-5','eng-240-pdf-export',NULL),
(9,'ENG-241','implement','implement','held','later','payments','batch','seed-9','acme/shop','Email a monthly invoice digest',strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),'claude-sonnet-5-5','eng-241-digest','waits for ENG-240 to merge');
UPDATE tasks SET batch = 1, batch_entry = 'e1' WHERE id = 5;
UPDATE tasks SET batch = 1, batch_entry = 'e2' WHERE id = 7;
UPDATE tasks SET batch = 1, batch_entry = 'e3' WHERE id = 8;
UPDATE tasks SET batch = 1, batch_entry = 'e4' WHERE id = 9;
INSERT INTO batches (id,key,task,title,repo,band,lane,issues,entries_workflow,state,plan,plan_revision,asked_revision,went,went_by) VALUES
(1,'linear:ENG-238',6,'Invoice exports','acme/shop','later','payments','["ENG-201","ENG-239","ENG-240","ENG-241"]','implement','going',
'{"title":"Invoice exports","entries":[{"id":"e1","issue":"ENG-201","title":"Add the invoices table","brief":"Add the table and its migration.","files":["migrations/"],"after":[]},{"id":"e2","issue":"ENG-239","title":"Export invoices as CSV","brief":"Stream the rows as CSV.","after":[{"entry":"e1","moment":"merged","why":"it reads the new table"}]},{"id":"e3","issue":"ENG-240","title":"Export invoices as PDF","brief":"Render one invoice per page.","after":[{"entry":"e1","moment":"merged","why":"it reads the new table"}]},{"id":"e4","issue":"ENG-241","title":"Email a monthly invoice digest","brief":"Attach the PDF export.","after":[{"entry":"e3","moment":"merged","why":"it attaches the PDF"}]}],"close":[]}',
1,1,strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'cli');
INSERT INTO pull_requests (task,resource,branch,base,state) VALUES (5,'github:acme/shop#198','eng-201-invoices-table','main','merged'),(1,'github:acme/shop#212','eng-212-retry-invoice-emails','main','open');
-- asks
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,tradeoff,title,text,what_waits,next_action,created) VALUES
(1,1,'approve-pr','step',1,'Open the pull request for ENG-212?','["open","stop"]','open','The retry loop adds 40 lines to the mailer, and the change is covered by 6 new tests.','Open the pull request for ENG-212','Checks are green at a1f3c9e.'||char(10)||char(10)||'- Failed invoice emails retry 3 times, with a 1, 5 and 25 minute backoff.'||char(10)||'- A message that still fails lands in the invoice_email_failures table.'||char(10)||'- The mailer tests pass: 48 passed, 0 failed.','The pull request waits on your answer.','Priory opens the pull request on acme/shop.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-40 minutes')),
(2,2,'approve-pr','step',1,'Open the pull request for ENG-219?','["open","stop"]','open','The PDF layout shifts by 2 mm when the VAT number is long.','Open the pull request for ENG-219','The VAT number prints under the billing address. A snapshot test covers both a short and a long number.','The pull request waits on your answer.','Priory opens the pull request.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours')),
(3,3,'implement','agent',2,'Which staging cluster should the search tests use?'||char(10)||'The default one refused the connection twice.','["use staging-b","skip the integration tests","wait for staging-a"]','use staging-b','staging-b runs an older index version, so two tests may need a rerun.','Pick a search cluster for the tests','The integration tests for tag indexing need a live cluster.','The run is paused on the tests.','The agent reruns the suite against the cluster you pick.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours')),
(4,4,'implement','agent',1,'Should the order history cursor encode the order id or the created time?','["order id","created time"]','order id','Ids are unique, so the cursor stays stable when two orders share a second.','Choose the pagination cursor','Both work. The id is stable, the time is easier to read in logs.','The run waits on the cursor.','The agent finishes the endpoint.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-12 minutes'));
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,answer,answered,answered_by,title,created) VALUES
(5,5,'approve-pr','step',1,'Open the pull request for ENG-201?','["open","stop"]','open','open',strftime('%Y-%m-%dT%H:%M:%SZ','now','-27 hours'),'serve','Open the pull request for ENG-201',strftime('%Y-%m-%dT%H:%M:%SZ','now','-28 hours'));
-- ENG-231 answers asks itself at high: one answered by auto, one from a session, one by a person
UPDATE tasks SET auto_answer = 'high' WHERE id = 4;
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,tradeoff,title,answer,answered,answered_by,confidence,rationale,created) VALUES
(41,4,'implement','agent',1,'When a client sends no cursor, should GET /orders return every order, as today, or the first page?','["every order, as today","the first page"]','every order, as today','Large merchants keep timing out until their clients send a cursor.','Keep the full response for clients without a cursor','every order, as today',strftime('%Y-%m-%dT%H:%M:%SZ','now','-38 minutes'),'auto','high','The brief asks for it in so many words, and 14 internal callers send no cursor yet.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-38 minutes')),
(42,4,'implement','agent',1,'Should the cursor parameter be called after or cursor?','["after","cursor"]','after',NULL,'Name the cursor parameter','after',strftime('%Y-%m-%dT%H:%M:%SZ','now','-60 minutes'),'session:7f3c2a','medium','The invoices endpoint already uses after.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes')),
(43,4,'implement','agent',1,'How many orders should a page hold by default?','["25","40","50"]','50',NULL,'Pick the default page size','40',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'serve','medium','The brief says 50 a page, and the slowest merchant returns 50 orders in 180 ms.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-130 minutes'));
-- holds for the batch entries
INSERT INTO holds (task,reason,kind,target,resume,created) VALUES (7,'waits for ENG-201 to merge','merged',5,'queued',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours')),(9,'waits for ENG-240 to merge','merged',8,'queued',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'));
-- inbox: unrouted
INSERT INTO inbox (id,type,subject,time,data,links,outcome) VALUES
('u1','github.pull_request_review_comment.created','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-50 minutes'),'{"repo":"acme/shop","pull_request":212,"body":"Can the backoff be configurable?","url":"https://github.com/acme/shop/pull/212#c1","actor":"reviewer-a","association":"MEMBER"}','[]','ignored'),
('u2','github.check_run.completed','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-45 minutes'),'{"repo":"acme/shop","pull_request":212,"title":"lint: success","url":"https://github.com/acme/shop/pull/212/checks"}','[]','ignored'),
('u3','github.issue_comment.created','acme/shop#198',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'{"repo":"acme/shop","issue":198,"body":"Nice, the export can build on this.","url":"https://github.com/acme/shop/pull/198#c2","actor":"reviewer-b","association":"MEMBER"}','[]','ignored'),
('u4','linear.comment.created','ENG-224',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'{"issue":"ENG-224","body":"staging-a is down until 15:00.","actor":"ops-a"}','[]','ignored'),
('u5','github.issue_comment.created','acme/shop#150',strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 hours'),'{"repo":"acme/shop","issue":150,"body":"Is this still planned?","url":"https://github.com/acme/shop/issues/150#c3"}','[]','ignored'),
('p1','github.pull_request.opened','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),'{"repo":"acme/shop","pull_request":212,"head_sha":"a1f3c9e5b2d4","head_branch":"eng-212-retry-invoice-emails","base_branch":"main","url":"https://github.com/acme/shop/pull/212","title":"Retry failed invoice emails with backoff","state":"open"}','[]','routed'),
('p2','github.check_run.completed','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'{"repo":"acme/shop","pull_request":212,"head_sha":"a1f3c9e5b2d4","head_branch":"eng-212-retry-invoice-emails","base_branch":"main","name":"test","conclusion":"success","title":"Retry failed invoice emails with backoff","state":"open"}','[]','routed'),
('p3','github.check_run.completed','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'{"repo":"acme/shop","pull_request":212,"head_sha":"a1f3c9e5b2d4","head_branch":"eng-212-retry-invoice-emails","base_branch":"main","name":"lint","conclusion":"success","title":"Retry failed invoice emails with backoff","state":"open"}','[]','routed'),
('o1','github.issue_comment.created','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),'{"repo":"acme/shop","issue":212,"body":"Please also retry SMS receipts.","actor":"outside-user","association":"NONE"}','[]','outsider');
UPDATE inbox SET data = json_set(data,'$.pull_request',150) WHERE id='u5';
UPDATE inbox SET data = json_remove(json_set(data,'$.pull_request',198),'$.issue') WHERE id='u3';
-- trace: costs over 14 days
WITH RECURSIVE d(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM d WHERE n < 13)
INSERT INTO trace (id,type,time,event)
SELECT lower(hex(randomblob(16))), 'factory.step.ended', strftime('%Y-%m-%dT%H:%M:%fZ','now',printf('-%d days',n),printf('-%d hours',k)),
 json_object('data', json_object('workflow', CASE WHEN k%3=0 THEN 'plan-batch' ELSE 'implement' END, 'task', '1','step','implement','decider','model','outcome','passed',
  'model', CASE WHEN k%3=0 THEN 'claude-opus-5-5' ELSE 'claude-sonnet-5-5' END, 'machine', CASE WHEN k%2=0 THEN 'dev-box' ELSE 'build-host' END,
  'tokens', json_object('input', 40000+((n*7+k*13)%9)*9000, 'output', 12000+((n*5+k*11)%7)*3000, 'cache_read', 600000+((n*3+k)%8)*120000, 'cache_write', 90000+((n+k)%5)*20000)))
FROM d, (SELECT 1 AS k UNION SELECT 2 UNION SELECT 3 UNION SELECT 4) WHERE (n+k)%5 != 0;
UPDATE tasks SET state='ready', reason=NULL, step='implement' WHERE id=7;
UPDATE holds SET met=strftime('%Y-%m-%dT%H:%M:%SZ','now','-26 hours'), met_as='merged', released=strftime('%Y-%m-%dT%H:%M:%SZ','now','-26 hours'), released_as='met' WHERE task=7;
INSERT OR IGNORE INTO owns (task, resource) VALUES (1,'github:acme/shop#212'),(5,'github:acme/shop#198'),(3,'linear:ENG-224');
INSERT OR IGNORE INTO owns (task, resource) SELECT id, key FROM tasks;
UPDATE inbox SET task = 1 WHERE id IN ('p1','p2','p3');
WITH RECURSIVE h(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM h WHERE n < 11)
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch)
SELECT 20+n, printf('ENG-%d',180+n), 'implement','done','done','today', CASE WHEN n%2=0 THEN 'payments' ELSE 'search' END,'linear',printf('h%d',n),'acme/shop',
 CASE n%4 WHEN 0 THEN 'Fix the rounding of invoice totals' WHEN 1 THEN 'Cache the product tag lookup' WHEN 2 THEN 'Add a retry to the webhook sender' ELSE 'Remove the unused coupon column' END,
 strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours', 20+n*22)), strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours', 18+n*22)), strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours', 18+n*22)),'claude-sonnet-5-5', printf('eng-%d',180+n) FROM h;
UPDATE tasks SET created = strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours', 20+(id-20)*22)) WHERE id BETWEEN 20 AND 31;
INSERT INTO owns (task,resource) SELECT id, printf('github:acme/shop#%d',160+id-20) FROM tasks WHERE id BETWEEN 20 AND 31;
INSERT INTO inbox (id,type,subject,time,data,links,outcome,task)
SELECT printf('m%d',id),'github.pull_request.closed',printf('acme/shop#%d',160+id-20), strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours', 18+(id-20)*22)),
 json_object('repo','acme/shop','pull_request',160+id-20,'merged',json('true'),'head_sha',printf('%07x',id*7919),'head_branch',printf('eng-%d',180+id-20),'base_branch','main'),'[]','stored',id FROM tasks WHERE id BETWEEN 20 AND 31;
INSERT INTO asks (task,step,kind,question,options,created,title)
SELECT id,'approve-pr','step','Open the pull request?','["open","stop"]', strftime('%Y-%m-%dT%H:%M:%SZ','now',printf('-%d hours',19+(id-20)*22)),'Open the pull request' FROM tasks WHERE id BETWEEN 20 AND 31;
UPDATE ask_revisions SET created = (SELECT created FROM asks WHERE asks.id = ask_revisions.ask) WHERE ask IN (SELECT id FROM asks WHERE task BETWEEN 20 AND 31);
UPDATE asks SET answer='open', answered_by='serve', answered = strftime('%Y-%m-%dT%H:%M:%SZ', created, printf('+%d minutes', 3+(id%5)*4)) WHERE task BETWEEN 20 AND 31;
-- runs, for the Runs panel: each ended run's result and its step.ended event with model and tokens
INSERT INTO runs (id,task,step,tmux,session,agent,started,ended,ended_as) VALUES
(101,6,'plan','demo','demo-101','planner',strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-288 minutes'),'done'),
(102,1,'implement','demo','demo-102','implementer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-4 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-189 minutes'),'moved'),
(103,3,'implement','demo','demo-103','implementer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-206 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),'blocked'),
(104,2,'implement','demo','demo-104','implementer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-157 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'moved'),
(105,8,'implement','demo','demo-105','implementer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-62 minutes'),'exited');
INSERT INTO results (run,task,step,outcome,summary) VALUES
(101,6,'plan','approved','Planned four entries'),(102,1,'implement','passed','Retries with backoff'),
(103,3,'implement','blocked','The staging search cluster refused the connection'),(104,2,'implement','passed','VAT number on the PDF');
INSERT INTO trace (id,type,time,event)
SELECT 'demo-run-' || id, 'factory.step.ended', ended, json_object('data', json_object('run', CAST(id AS TEXT), 'step', step,
  'model', CASE WHEN id IN (101,103) THEN 'claude-opus-5-5' ELSE 'claude-sonnet-5-5' END,
  'tokens', json_object('input', 30000+id%7*41000, 'output', 9000+id%5*3000, 'cache_read', 120000+id%4*90000, 'cache_write', 20000)))
FROM runs WHERE id BETWEEN 101 AND 105;
-- review feedback on task 1's pull request: a review the intake read without the branches, and the agent's sorting
INSERT INTO inbox (id,type,subject,time,data,links,outcome,task) VALUES
('p4','github.pull_request_review.submitted','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-150 minutes'),'{"repo":"acme/shop","pull_request":212,"head_sha":"a1f3c9e5b2d4","head_branch":null,"base_branch":null,"state":"changes_requested","body":"Two things.","url":"https://github.com/acme/shop/pull/212#pullrequestreview-301","actor":"reviewer-a","association":"MEMBER"}','[]','routed',1),
('p5','linear.issue.updated','ENG-212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-140 minutes'),'{"issue":"ENG-212","title":"Retry failed invoice emails"}','[]','routed',1);
INSERT INTO feedback_triage (task,item,class,run) VALUES
(1,'https://github.com/acme/shop/pull/212#discussion_r802','trivial',102),
(1,'https://github.com/acme/shop/pull/212#discussion_r803','nothing-to-do',102),
(1,'https://github.com/acme/shop/pull/212#pullrequestreview-301','persons-call',102);
-- a second and third runner: this state is workstation's, laptop is live, desktop was last seen 40 minutes ago
INSERT INTO runner (id,name) VALUES (1,'workstation');
INSERT INTO leases (runner,alive) VALUES ('workstation',unixepoch('now')),('laptop',unixepoch('now','-1 minutes')),('desktop',unixepoch('now','-40 minutes'));
INSERT INTO remote_asks (runner,ask,version,data) VALUES
('desktop',12,1,json_object('id',12,'task',4,'key','ENG-260','step','approve-pr','kind','step','lane','payments','band','now','state','open','revision',1,'since',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'title','Merge the rate limiter','question','Merge the rate limiter?','text','The checkout webhook now drops calls past 20 a second per merchant.','options',json('["approved","rejected"]'),'recommendation','approved','free_text',json('false'),'risk',json('[]'),'refused',json('[]'),'command',NULL,'answer',NULL,'answered_at',NULL)),
('laptop',3,1,json_object('id',3,'task',2,'key','ENG-262','step','deploy','kind','step','lane','payments','band','now','state','open','revision',1,'since',strftime('%Y-%m-%dT%H:%M:%SZ','now','-12 minutes'),'title','Deploy to staging','question','Deploy to staging?','text','The checks passed on the merge commit.','options',json('["approved","rejected"]'),'recommendation','approved','free_text',json('false'),'risk',json('[]'),'refused',json('[]'),'command',json_object('argv',json('["./deploy","staging"]'),'target','shell','dir','/home/max/acme/shop','runs',json('["approved"]')),'answer',NULL,'answered_at',NULL)),
('laptop',4,2,json_object('id',4,'task',5,'key','ENG-263','step','plan','kind','agent','lane','search','band','today','state','open','revision',2,'since',strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'),'title','Split the migration','question','Split the migration in two pull requests?','text','The index migration and the backfill touch different tables.','options',json('["split it","keep one","ask the team"]'),'recommendation',NULL,'risk',json('[]'),'refused',json('[]'),'command',NULL,'answer',NULL,'answered_at',NULL));
INSERT INTO remote_submissions (submission,runner,ask,revision,verb,text,sent,outcome,why,applied) VALUES
('0193f0a2-demo-0001','desktop',12,1,'answer','approved',strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 minutes'),NULL,NULL,NULL),
('0193f0a2-demo-0002','laptop',4,1,'answer','keep one',strftime('%Y-%m-%dT%H:%M:%SZ','now','-9 minutes'),'refused','ask 4 is at revision 2',strftime('%Y-%m-%dT%H:%M:%SZ','now','-8 minutes'));
-- two tasks the other runners hold, from their claims: laptop keeps one, desktop waits on one
INSERT INTO inbox (id,type,subject,time,data,links,outcome) VALUES
('r-262','priory.task.requested','ENG-262',strftime('%Y-%m-%dT%H:%M:%SZ','now','-50 minutes'),'{"key":"ENG-262","workflow":"implement","band":"now","lane":"payments","tldr":"Deploy the rate limiter to staging"}','[]','ignored'),
('r-264','priory.task.requested','ENG-264',strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 minutes'),'{"key":"ENG-264","workflow":"implement","band":"today","lane":"search","tldr":"Reindex the product catalog"}','[]','ignored'),
('c-262','priory.task.claimed','ENG-262',strftime('%Y-%m-%dT%H:%M:%SZ','now','-48 minutes'),'{"key":"ENG-262","runner":"laptop"}','[]','ignored'),
('c-264','priory.task.claimed','ENG-264',strftime('%Y-%m-%dT%H:%M:%SZ','now','-28 minutes'),'{"key":"ENG-264","runner":"desktop"}','[]','ignored');
INSERT INTO claims (key,runner,request,hold,won,generation) VALUES
('ENG-262','laptop','r-262','kept',(SELECT seq FROM inbox WHERE id='c-262'),2),
('ENG-264','desktop','r-264','waiting',(SELECT seq FROM inbox WHERE id='c-264'),1);
-- a cancel sent to desktop's task, which desktop has not read yet
INSERT INTO remote_task_submissions (submission,runner,key,generation,act,sent) VALUES
('0193f0a2-demo-0003','desktop','ENG-264',1,'cancel',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 minutes'));
-- events a task kept that no agent read: an asleep task, the blocked task 3 and a done task
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch) VALUES
(10,'linear:ENG-245','implement','implement','asleep','today','payments','linear','seed-10','acme/shop','Retry webhook deliveries',strftime('%Y-%m-%dT%H:%M:%SZ','now','-6 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-90 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-90 minutes'),'claude-sonnet-5-5','eng-245'),
(11,'github:acme/shop#190','implement','done','done','later','payments','github','seed-11','acme/shop','Add a currency column',strftime('%Y-%m-%dT%H:%M:%SZ','now','-20 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-5 hours'),'claude-sonnet-5-5','eng-190');
INSERT OR IGNORE INTO owns (task, resource) VALUES (10,'linear:ENG-245'),(11,'github:acme/shop#190');
INSERT INTO inbox (id,type,subject,time,data,links,outcome,task) VALUES
('k1','linear.comment.created','ENG-245',strftime('%Y-%m-%dT%H:%M:%SZ','now','-40 minutes'),'{"issue":"ENG-245","body":"Use exponential backoff, capped at 10 minutes.","actor":"lead-a"}','[]','kept',10),
('k2','github.pull_request_review.submitted','acme/shop#245',strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'),'{"repo":"acme/shop","pull_request":245,"state":"commented","body":"Looks good once the cap is configurable.","url":"https://github.com/acme/shop/pull/245#pullrequestreview-9","actor":"reviewer-a"}','[]','kept',10),
('k3','linear.comment.created','ENG-224',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes'),'{"issue":"ENG-224","body":"staging-b is back up, use it.","actor":"ops-a"}','[]','kept',3),
('k4','github.pull_request.comment','acme/shop#190',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),'{"repo":"acme/shop","pull_request":190,"body":"This broke the CSV export of old invoices.","url":"https://github.com/acme/shop/pull/190#c4","actor":"reviewer-b"}','[]','kept',11);
-- two memory candidates that wait for a person: one replaces the factory's release-day, one is new
INSERT INTO memory_candidates (id,name,description,text,key,runner,written,logged) VALUES
('0193f0a2-mem-0001','release-day','When releases go out. Read it before you plan or merge a change near a release.','Releases go out on Wednesdays since November 2026, not Tuesdays. Merge nothing risky on a Tuesday afternoon.'||char(10)||char(10)||'The release train freezes main at 14:00 UTC, so a pull request merged after that waits a week.','ENG-212','laptop',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),1),
('0193f0a2-mem-0002','search-staging','Which search cluster the integration tests use.','staging-a refuses connections from runners outside the VPN. Run the search integration tests against staging-b, and rerun the two index-version tests once if they fail.','ENG-224',NULL,strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'),2);
-- scores: two scorers on the demo's runs, and a week of scores across workflows for the Costs panel
INSERT INTO trace (id,type,time,event)
SELECT 'demo-score-' || r.run || '-' || s.scorer, 'factory.run.scored', strftime('%Y-%m-%dT%H:%M:%fZ','now','-50 minutes'),
 json_object('data', json_object('workflow','implement','task',CAST(r.task AS TEXT),'run',CAST(r.run AS TEXT),'step',r.step,'machine','dev-box',
  'scorer',s.scorer,'kind',s.kind,'verdict',
   CASE WHEN s.scorer='gate' THEN r.gate ELSE r.passed END,
  'reason', CASE WHEN s.scorer='gate' THEN r.gate_reason ELSE r.passed_reason END))
FROM (SELECT 102 AS run, 1 AS task, 'implement' AS step, 'pass' AS gate, 'the tests pass' AS gate_reason, 'pass' AS passed, 'outcome passed' AS passed_reason
      UNION ALL SELECT 103,3,'implement','error','timed out after 600s','fail','outcome blocked, not passed'
      UNION ALL SELECT 104,2,'implement','fail','2 of 214 tests fail: test_vat_rounding, test_pdf_footer','pass','outcome passed'
      UNION ALL SELECT 101,6,'plan','not applicable','the result names no commit','not applicable','a plan step has no passed outcome') r,
     (SELECT 'gate' AS scorer, 'command' AS kind UNION ALL SELECT 'passed','outcome') s;
WITH RECURSIVE d(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM d WHERE n < 17)
INSERT INTO trace (id,type,time,event)
SELECT 'demo-score-week-' || n || '-' || s.scorer, 'factory.run.scored', strftime('%Y-%m-%dT%H:%M:%fZ','now',printf('-%d hours', 2+n*7)),
 json_object('data', json_object('workflow', CASE WHEN n%3=0 THEN 'plan-batch' ELSE 'implement' END,'task','1','run',CAST(200+n AS TEXT),'step','implement',
  'machine', CASE WHEN n%2=0 THEN 'dev-box' ELSE 'build-host' END,'scorer',s.scorer,'kind',s.kind,
  'verdict', CASE WHEN (n*s.k)%7=3 THEN 'fail' WHEN (n+s.k)%9=4 THEN 'error' WHEN n%3=0 AND s.scorer='gate' THEN 'not applicable' ELSE 'pass' END,'reason','seeded'))
FROM d, (SELECT 'gate' AS scorer,'command' AS kind, 1 AS k UNION ALL SELECT 'passed','outcome',2) s;
-- task 4's request, brief and plan, as Markdown: the task page shows its Brief and Plan
INSERT INTO inbox (id,type,subject,time,data,links,outcome,task) VALUES
('seed-4','linear.issue.created','ENG-231',strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),json_object('issue','ENG-231','title','Paginate the order history endpoint','description','`GET /orders` returns every order at once, and large merchants time out.'||char(10)||char(10)||'- Return 50 orders a page.'||char(10)||'- Keep the old response for clients that send no cursor.'),'[]','routed',4);
INSERT INTO runs (id,task,step,tmux,session,agent,started,ended,ended_as) VALUES
(106,4,'groom','demo','demo-106','groomer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-170 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-160 minutes'),'moved'),
(107,4,'plan','demo','demo-107','planner',strftime('%Y-%m-%dT%H:%M:%SZ','now','-150 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-130 minutes'),'moved');
INSERT INTO results (run,task,step,outcome,summary) VALUES
(106,4,'groom','passed','Paginate `GET /orders` with an opaque cursor.'||char(10)||char(10)||'**Acceptance**'||char(10)||char(10)||'1. A request with no cursor gets the first 50 orders and a `next` cursor.'||char(10)||'2. A request with the last cursor gets an empty page and no `next`.'||char(10)||'3. Clients that send no `limit` keep working.'),
(107,4,'plan','approved','1. Add a `cursor` and `limit` to the orders query in `orders/list.rs`.'||char(10)||'2. Encode the cursor as the last order id, base64.'||char(10)||'3. Cover the three acceptance cases in `tests/orders.rs`.'||char(10)||char(10)||'See [the API guide](https://example.com/api/pagination) for the cursor format.');
-- a live run: task 8's implement step is working now
UPDATE tasks SET state='running', changed=strftime('%Y-%m-%dT%H:%M:%SZ','now','-6 minutes') WHERE id=8;
INSERT INTO runs (id,task,step,tmux,session,agent,started) VALUES (108,8,'implement','demo','demo-108','implementer',strftime('%Y-%m-%dT%H:%M:%SZ','now','-6 minutes'));
-- a command ask on this runner: its answer runs a deploy
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch) VALUES
(12,'ENG-250','implement','deploy','asleep','now','payments','linear','seed-12','acme/shop','Deploy the invoice retries to staging',strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-8 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-8 minutes'),'claude-sonnet-5-5','eng-250');
INSERT OR IGNORE INTO owns (task, resource) VALUES (12,'linear:ENG-250');
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,title,text,command,created) VALUES
(40,12,'deploy','step',1,'Deploy ENG-250 to staging?','["deploy","wait"]','deploy','Deploy to staging','The merge commit passed every check.','{"argv":["./deploy","staging","--service","mailer"],"target":"shell","dir":"/home/max/acme/shop","runs":["deploy"],"timeout":null}',strftime('%Y-%m-%dT%H:%M:%SZ','now','-8 minutes'));
-- an approval step's untitled ask with a 12-line question: the detail heads it with a short title
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch) VALUES
(13,'ENG-252','implement','approve-implementation','asleep','today','payments','linear','seed-13','acme/shop',NULL,strftime('%Y-%m-%dT%H:%M:%SZ','now','-90 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'),'claude-opus-5-5','eng-252');
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,created) VALUES
(90,13,'approve-implementation','step',1,'approve-implementation: Plan: /home/max/acme/shop/.priory/plans/ENG-252-plan.md. Only apps/admin-api/src/mcp/routes.ts changes.'||char(10)||'The route table gains one entry for the export tool, behind the existing admin guard.'||char(10)||'No migration: the export reads the tables the list tools already read.'||char(10)||'Tests: apps/admin-api/test/mcp/routes.test.ts adds a case for the guard and one for the export.'||char(10)||'The plan leaves the client untouched; the next step regenerates its types.'||char(10)||'Risk: the guard is shared with the list tools, so a mistake there would expose them too.'||char(10)||'I checked acme/shop#198, which added the guard, for its tests.'||char(10)||'They cover an admin and a viewer; the export adds a third case for a revoked token.'||char(10)||'The change is about 40 lines with tests.'||char(10)||'Approve to start the implementation run, or revise to send the plan back.'||char(10)||'Reject drops the task.'||char(10)||'Nothing else waits on this answer.','["approve","revise","reject"]','approve',strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 minutes'));
-- a groom step's plan approval, with the plan stored when it was asked
INSERT INTO tasks (id,key,workflow,step,state,band,lane,trigger,source,repo,tldr,created,changed,entered,model,branch) VALUES
(14,'ENG-270','groom','approve-plan','asleep','today','search','linear','seed-14','acme/shop','Paginate the search results page',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes'),strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes'),'claude-opus-5-5','priory/eng-270');
INSERT INTO asks (id,task,step,kind,revision,question,options,recommendation,title,text,created) VALUES
(91,14,'approve-plan','step',1,'Approve the plan for ENG-270?','["approve","revise","reject"]','approve','Approve the plan for ENG-270','The brief holds three decisions for you, the acceptance criteria and the plan.',strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes'));
INSERT INTO ask_plans (ask,step,run,files,contents) VALUES
(91,'groom',0,json_array(json_object('path','/home/me/worktrees/acme/shop/eng-270/.priory/brief.md','text','# ENG-270: paginate the search results page

## Decisions to approve

- **D1.** Page with an opaque cursor, not an offset, so new products never shift a page.
- **D2.** Show 24 results a page, the grid''s three full rows on a desktop.
- **D3.** Keep the `?page=` links working: redirect them to the first page.

## Acceptance criteria

1. A search with no cursor shows the first 24 results and a Next link.
2. The last page shows no Next link.
3. An old `?page=3` link lands on the first page with a 301.

## Plan

1. Add `cursor` and `limit` to `search/query.rs`, keyed on the result''s rank and id.
2. Render Next and Previous links in `templates/search.html`.
3. Cover the three criteria in `tests/search_pages.rs`.

Risk: the rank changes when the index rebuilds, so a cursor can skip a result during a rebuild.
','truncated',json('false'))),'[null,null]');
-- ask 2 snoozed until tomorrow 09:00
INSERT INTO snoozes (kind,item,task,snoozed,until) VALUES ('ask',2,2,strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 minutes'),strftime('%Y-%m-%dT09:00:00Z','now','+1 day'));
-- a stalled queue's banner, and a skipped check on task 1's pull request
INSERT INTO stalls (resource,missing,task,since) VALUES ('disk','2 GB',4,strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes'));
INSERT INTO inbox (id,type,subject,time,data,links,outcome,task) VALUES
('p6','github.check_run.completed','acme/shop#212',strftime('%Y-%m-%dT%H:%M:%SZ','now','-140 minutes'),'{"repo":"acme/shop","pull_request":212,"head_sha":"a1f3c9e5b2d4","head_branch":"eng-212-retry-invoice-emails","base_branch":"main","name":"e2e","conclusion":"skipped","title":"Retry failed invoice emails with backoff","state":"open"}','[]','routed',1);
-- each open ask's revision was asked when the ask was
UPDATE ask_revisions SET created = (SELECT created FROM asks WHERE asks.id = ask_revisions.ask) WHERE ask IN (1,2,3,4,40,90,91);
-- a request a trigger's pre script dropped: the Inbox's Dropped section and the Desk's row
INSERT INTO inbox (id,type,subject,time,data,links,outcome) VALUES
('d1','linear.issue.entered','ENG-241',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes'),'{"identifier":"ENG-241","title":"Speed up the CSV export","url":"https://linear.app/acme/issue/ENG-241"}','[]','ignored'),
('d2','factory.intake.dropped','linear:ENG-241',strftime('%Y-%m-%dT%H:%M:%SZ','now','-70 minutes'),'{"trigger":"linear-view","script":"scripts/has-estimate","reason":"no estimate yet","failed":null,"key":"linear:ENG-241","source":"d1"}','[]','ignored');
-- a report with a bare URL, as the daemon writes one when a landed change goes red
INSERT INTO inbox (id,type,subject,time,data,links,outcome) VALUES
('landed-1','priory.report','priory',strftime('%Y-%m-%dT%H:%M:%SZ','now','-12 minutes'),'{"line":"landed wrong: acme/shop#198 of task 20 is red on main: deploy cancelled https://github.com/acme/shop/actions/runs/1001"}','[]','stored');
