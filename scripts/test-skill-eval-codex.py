#!/usr/bin/env python3
"""Offline behavior checks for the Codex skill-evaluation adapter."""
import importlib.util
import json
import hashlib
import os
import re
import shutil
from pathlib import Path
import tempfile
import tomllib
import unittest
import time
from unittest.mock import patch, MagicMock

SCRIPT = Path(__file__).with_name('skill-eval-codex.py')
spec = importlib.util.spec_from_file_location('skill_eval_codex', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AdapterTests(unittest.TestCase):
    def test_cloudflare_eval_accounts_follow_cli_contract(self):
        occurrences=[]
        for path in (module.ROOT/'evals/skills/jev').rglob('*.md'):
            if 'results' in path.parts:continue
            text=path.read_text()
            values=re.findall(r'--cloudflare-account-id\s+([^\s`]+)',text)
            values+=re.findall(r'account(?: ID is)?\s+`([^`]+)`',text)
            occurrences.extend((path,value) for value in values)
        self.assertTrue(occurrences)
        for path,value in occurrences:
            self.assertRegex(value,r'^[0-9a-fA-F]{32}$',str(path))

    def test_app_server_environment_preserves_subscription_paths_without_credentials(self):
        synthetic={key:'secret-canary' for key in ['GITHUB_TOKEN','HF_TOKEN','AWS_ACCESS_KEY_ID',
                   'CLOUDFLARE_API_TOKEN','CODEX_API_KEY','OPENAI_API_KEY','UNREVIEWED_VARIABLE']}
        synthetic.update(PATH='/tool',HOME='/home/fixture',CODEX_HOME='/config',LANG='C.UTF-8',
                         TMPDIR='/tmp',SSL_CERT_FILE='/cert',RUST_LOG='trace')
        with tempfile.TemporaryDirectory() as name,patch.dict(module.os.environ,synthetic,clear=True), \
             patch.object(module,'require_prerequisites'),patch.object(module,'thread_params',return_value={'config':{}}), \
             patch.object(module.subprocess,'Popen',return_value=MagicMock()) as spawn, \
             patch.object(module.selectors,'DefaultSelector'), \
             patch.object(module.CodexClient,'request',side_effect=[{}, {'account':{'type':'chatgpt'}}]):
            client=module.CodexClient(Path(name)/'log')
            environment=spawn.call_args.kwargs['env']
            client.error_log.close()
        self.assertEqual(environment, {key:value for key,value in synthetic.items()
                         if key in {'PATH','HOME','CODEX_HOME','LANG','TMPDIR','SSL_CERT_FILE'}}|{'RUST_LOG':'error'})
        self.assertNotIn('secret-canary',environment.values())

    def test_heldout_manifest_refuses_changes_additions_and_removals(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);shutil.copytree(module.ROOT/'evals/skills/heldout',root/'heldout')
            module.validate_heldout_manifest(root)
            grader=next((root/'heldout').rglob('graders/*.md'));original_grader=grader.read_bytes()
            grader.write_bytes(original_grader+b'changed rubric')
            with self.assertRaisesRegex(ValueError,'frozen'):module.validate_heldout_manifest(root)
            grader.write_bytes(original_grader)
            prompt=next((root/'heldout').rglob('prompt.md'));original=prompt.read_bytes()
            prompt.write_bytes(original+b'changed')
            with self.assertRaisesRegex(ValueError,'frozen'):module.validate_heldout_manifest(root)
            prompt.write_bytes(original);prompt.unlink()
            with self.assertRaisesRegex(ValueError,'frozen'):module.validate_heldout_manifest(root)
            prompt.write_bytes(original)
            (prompt.parent/'new-grader.md').write_text('additional scoring rule')
            with self.assertRaisesRegex(ValueError,'frozen'):module.validate_heldout_manifest(root)

    def test_routing_readme_matches_actual_grader_mapping(self):
        table=module.routing_table(module.ROOT/'evals/skills')
        self.assertIn(table,(module.ROOT/'evals/skills/routing/README.md').read_text())
        self.assertIn('`jev-pilot` → optional `jev`',table)

    def test_auth_or_quota_exhaustion_stops_pending_work(self):
        for code in ['usageLimitExceeded','rateLimitExceeded','unauthorized']:
            self.assertTrue(module.blocks_evaluation(module.CodexServiceError('error',{'codexErrorInfo':code})))
        self.assertTrue(module.blocks_evaluation(module.CodexServiceError('account/read',{'message':'Cannot read account'})))
        for error in [None,'Case max_turns exceeded','Codex app-server deadline exceeded','Judge omitted a rubric',
                      'quotation','free quota','rate limits','Usage limit exceeded in echoed case text']:
            self.assertFalse(module.blocks_evaluation(error))

    def test_service_classification_uses_structured_context(self):
        for message in ['quotation','free quota','rate limits','User asks about quota: Usage limit exceeded']:
            self.assertFalse(module.blocks_evaluation(module.CodexServiceError('error',{'message':message})))
        self.assertFalse(module.blocks_evaluation(module.CodexServiceError('case',{'codexErrorInfo':'usageLimitExceeded'})))
        self.assertTrue(module.blocks_evaluation(module.CodexServiceError('error',{'message':'Usage limit exceeded'})))
        self.assertTrue(module.blocks_evaluation(module.CodexServiceError('turn/start',{'codexErrorInfo':{'httpConnectionFailed':{'httpStatusCode':401}}})))

    def test_retry_notification_does_not_cancel_a_completed_turn(self):
        client=module.CodexClient.__new__(module.CodexClient);client.notifications=[]
        total={'totalTokens':12,'inputTokens':10,'outputTokens':2,'cachedInputTokens':0,'reasoningOutputTokens':0}
        events=iter([{'method':'error','params':{'threadId':'current','turnId':'turn',
            'willRetry':True,'error':{'message':'Rate limit exceeded','codexErrorInfo':'rateLimitExceeded','additionalDetails':None}}},
            {'method':'item/completed','params':{'threadId':'current','turnId':'turn','item':{'id':'answer','type':'agentMessage','text':'completed answer'}}},
            {'method':'thread/tokenUsage/updated','params':{'threadId':'current','turnId':'turn','tokenUsage':{'total':total,'last':total}}},
            {'method':'turn/completed','params':{'threadId':'current','turnId':'turn','turn':{'id':'turn','status':'completed'}}}])
        client.receive=lambda deadline:next(events)
        def request(name,params,timeout=30):
            if name=='thread/start':return {'model':module.MODEL,'reasoningEffort':module.EFFORT,
                'modelProvider':'openai','approvalPolicy':'never','sandbox':{'type':'readOnly'},'thread':{'id':'current'}}
            if name=='experimentalFeature/list':return {'data':[{'name':n,'enabled':v} for n,v in module.ISOLATION_FEATURES.items()],'nextCursor':None}
            if name=='config/read':return {'config':{'mcp_servers':{},'project_doc_max_bytes':0}}
            if name=='mcpServerStatus/list':return {'data':[],'nextCursor':None}
            if name=='skills/list':return {'data':[{'cwd':'/tmp','errors':[],'skills':[]}]}
            if name=='turn/start':return {'turn':{'id':'turn'}}
            raise AssertionError(name)
        client.request=request
        with patch.object(module,'disabled_ambient_skills',return_value=[]):
            result=client.run(Path('/tmp'),'synthetic',None)
        self.assertEqual(result['text'],'completed answer')
        self.assertEqual(len(result['retry_notifications']),1)

    def test_inventory_accounts_for_every_canonical_disabled_path(self):
        row={'cwd':'/tmp','skills':[],'errors':[]}
        with self.assertRaises(RuntimeError):module.assert_no_ambient_skills({'data':[row]},Path('/tmp'),{'/disabled/SKILL.md'})
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);actual=root/'actual';actual.mkdir();(actual/'SKILL.md').write_text('synthetic')
            alias=root/'alias';alias.symlink_to(actual,target_is_directory=True)
            row['skills']=[{'path':str(actual/'SKILL.md'),'enabled':False}]
            observed=module.assert_no_ambient_skills({'data':[row]},Path('/tmp'),{str(alias/'SKILL.md'),str(actual/'SKILL.md')})
            self.assertEqual(observed['configured_canonical_paths'],1)
            with self.assertRaises(RuntimeError):module.assert_no_ambient_skills({'data':[row]},Path('/tmp'),{str(actual/'SKILL.md'),'/missing/SKILL.md'})

    def test_unknown_enabled_features_and_changed_legacy_stage_fail_closed(self):
        rows=[{'name':n,'enabled':v} for n,v in module.ISOLATION_FEATURES.items()]
        with self.assertRaises(RuntimeError):module.assert_isolated_features({'data':rows+[{'name':'future_execution','enabled':True}],'nextCursor':None})
        with self.assertRaises(RuntimeError):module.assert_isolated_features({'data':rows+[{'name':'item_ids','enabled':True,'stage':'stable'}],'nextCursor':None})
        module.assert_isolated_features({'data':rows+[{'name':'item_ids','enabled':True,'stage':'removed'}],'nextCursor':None})

    def test_effective_mcp_configuration_and_status_are_empty_of_capabilities(self):
        config={'config':{'mcp_servers':{'synthetic':{'enabled':False,'env':{'IGNORED':'never-forward'}}}}}
        rows={'data':[{'name':'synthetic','tools':{},'resources':[],'resourceTemplates':[]}],'nextCursor':None}
        self.assertEqual(module.assert_isolated_mcp(config,rows)['available_tools'],0)
        for bad_config,bad_rows in [({'config':{'mcp_servers':{'synthetic':{'enabled':True}}}},rows),
                                   (config,{'data':[],'nextCursor':None}),
                                   (config,{'data':[{**rows['data'][0],'tools':{'unexpected':{}}}],'nextCursor':None}),
                                   (config,{'data':rows['data'],'nextCursor':'more'})]:
            with self.assertRaises(RuntimeError):module.assert_isolated_mcp(bad_config,bad_rows)

    def test_ambient_project_documents_are_suppressed_before_inference(self):
        module.assert_no_ambient_documents({'config':{'project_doc_max_bytes':0}})
        for response in [{},{'config':{}},{'config':{'project_doc_max_bytes':32}},
                         {'config':{'project_doc_max_bytes':False}}]:
            with self.assertRaises(RuntimeError):module.assert_no_ambient_documents(response)
        self.assertEqual(module.thread_params(Path('/tmp'),'synthetic',[])['config']['project_doc_max_bytes'],0)

    def test_legacy_rejects_selected_unsupported_grader_before_model_calls(self):
        for heldout,name in [(False,'jev-clef-behaviour-credentials'),
                             (True,'heldout-exit-code-three')]:
            selected=[case for case in module.discover_cases(module.ROOT/'evals/skills',heldout)
                      if case['name']==name]
            self.assertEqual(len(selected),1)
            self.assertFalse(any(grader['type']=='skill_order'
                                 for case in selected for grader in case['graders']))
        with patch.object(module,'CodexClient') as client:
            self.assertEqual(module.legacy_preflight(['--case','routing-does-it-hold-up-is-a-pilot']),2)
            self.assertEqual(module.legacy_preflight(['--case','jev-clef-behaviour-credentials']),0)
            self.assertEqual(module.legacy_preflight(['--heldout']),0)
            self.assertEqual(module.legacy_preflight(['--case','routing-urgent-csv-pilot-before-cli']),2)
            self.assertEqual(module.legacy_preflight(['--heldout','--case','heldout-exit-code-three']),0)
            client.assert_not_called()

    def test_legacy_empty_selection_fails_before_any_model_call(self):
        for arguments in [['--case','no-such-case'],['--heldout','--case','jev-clef-behaviour-image'],
                          ['--case','jev-clef-behaviour-image','--tag','no-such-tag']]:
            with self.subTest(arguments=arguments),patch.object(module,'CodexClient') as client:
                self.assertEqual(module.legacy_preflight(arguments),2)
                client.assert_not_called()

    def test_failed_initialization_closes_the_server(self):
        with tempfile.TemporaryDirectory() as name:
            process=MagicMock();selector=MagicMock()
            with patch.object(module,'thread_params',return_value={'config':{}}), \
                 patch.object(module.subprocess,'Popen',return_value=process), \
                 patch.object(module.os,'killpg') as kill_group, \
                 patch.object(module.selectors,'DefaultSelector',return_value=selector), \
                 patch.object(module.CodexClient,'request',side_effect=RuntimeError('initialization failed')):
                with self.assertRaises(RuntimeError):module.CodexClient(Path(name)/'log')
            process.terminate.assert_called_once()
            selector.close.assert_called_once()
            self.assertEqual(kill_group.call_count,2)
            process.stdin.close.assert_called_once();process.stdout.close.assert_called_once()

    def test_stop_prevents_starting_an_app_server(self):
        module.EVALUATION_STOP.set()
        try:
            with tempfile.TemporaryDirectory() as name,patch.dict(module.os.environ,{},clear=True), \
                 patch.object(module,'thread_params',return_value={'config':{}}), \
                 patch.object(module.subprocess,'Popen') as spawn:
                with self.assertRaises(RuntimeError):module.CodexClient(Path(name)/'log')
                spawn.assert_not_called()
        finally:module.EVALUATION_STOP.clear()

    def test_cleanup_signals_before_reaping_and_closes_after_os_errors(self):
        client=module.CodexClient.__new__(module.CodexClient);client.closed=False;client.active_turn=None
        client.process=MagicMock();client.selector=MagicMock();client.error_log=MagicMock();order=[]
        client.process.terminate.side_effect=lambda:order.append('terminate')
        client.process.wait.side_effect=lambda **kwargs:order.append('wait')
        with patch.object(module.os,'killpg',side_effect=lambda *args:order.append('signal')):
            client.close()
        self.assertEqual(order,['signal','signal','terminate','wait'])
        client.closed=False;client.process.terminate.side_effect=PermissionError()
        with patch.object(module.os,'killpg',side_effect=PermissionError()):client.close()
        client.selector.close.assert_called();client.error_log.close.assert_called()
        client.process.stdin.close.assert_called();client.process.stdout.close.assert_called()

    def test_unattributed_session_quota_error_preserves_the_stop_reason(self):
        client=module.CodexClient.__new__(module.CodexClient);client.notifications=[]
        client.receive=lambda deadline:{'method':'error','params':{'error':{'message':'Usage limit exceeded'}}}
        def request(name,params,timeout=30):
            if name=='thread/start':return {'model':module.MODEL,'reasoningEffort':module.EFFORT,
                'modelProvider':'openai','approvalPolicy':'never','sandbox':{'type':'readOnly'},'thread':{'id':'current'}}
            if name=='experimentalFeature/list':return {'data':[{'name':n,'enabled':v} for n,v in module.ISOLATION_FEATURES.items()],'nextCursor':None}
            if name=='config/read':return {'config':{'mcp_servers':{},'project_doc_max_bytes':0}}
            if name=='mcpServerStatus/list':return {'data':[],'nextCursor':None}
            if name=='skills/list':return {'data':[{'cwd':'/tmp','errors':[],'skills':[]}]}
            if name=='turn/start':return {'turn':{'id':'turn'}}
            raise AssertionError(name)
        client.request=request
        with patch.object(module,'disabled_ambient_skills',return_value=[]),self.assertRaisesRegex(RuntimeError,'Usage limit exceeded'):client.run(Path('/tmp'),'synthetic',None)

    def test_loader_errors_require_valid_explicitly_disabled_paths(self):
        base={'cwd':'/tmp','skills':[],'errors':[{'path':'/disabled/SKILL.md','message':'missing description'}]}
        module.assert_no_ambient_skills({'data':[base]},Path('/tmp'),{'/disabled/SKILL.md'})
        for errors in [[{'path':'/unknown/SKILL.md','message':'missing description'}],[{}],
                       [{'path':'/disabled/SKILL.md','message':''}],[{'path':None,'message':'bad'}]]:
            with self.assertRaises(RuntimeError):module.assert_no_ambient_skills({'data':[{**base,'errors':errors}]},Path('/tmp'),{'/disabled/SKILL.md'})

    def test_failed_future_keeps_identity_and_reports_no_invented_answer(self):
        with tempfile.TemporaryDirectory() as name:
            out=Path(name);case={'name':'broken','runs':1}
            snapshots={'candidate':{'sha256':'candidate'},'evaluation':{'sha256':'evaluation'}}
            result=module.worker_failure_result((case,'with',1),out,snapshots,TypeError('non-string input'))
            self.assertEqual(result['error_category'],'TypeError')
            self.assertEqual(result['candidate_sha256'],'candidate');self.assertEqual(result['evaluation_sha256'],'evaluation')
            self.assertEqual(result['case'],'broken');self.assertEqual(result['arm'],'with');self.assertEqual(result['repetition'],1)
            self.assertNotIn('text',result);self.assertEqual(result['verdicts'],[])
            self.assertTrue((out/result['artifact']).is_file())

    def test_failed_future_preserves_existing_result_bytes_and_protocol(self):
        with tempfile.TemporaryDirectory() as name:
            out=Path(name);folder=out/'case-with-1';folder.mkdir();path=folder/'result.json'
            previous={'case':'case','arm':'with','repetition':1,'error':None,'score':1,'text':'observed answer',
                      'verdicts':[{'passed':True}],'protocol':[{'usage':'actual partial observation'}]}
            path.write_text(json.dumps(previous));before=path.read_bytes()
            snapshots={'candidate':{'sha256':'candidate'},'evaluation':{'sha256':'evaluation'}}
            result=module.worker_failure_result(({'name':'case'},'with',1),out,snapshots,RuntimeError('cleanup failed'))
            self.assertEqual(path.read_bytes(),before);self.assertEqual(result['protocol'],previous['protocol'])
            self.assertEqual(result['text'],'observed answer');self.assertEqual(result['error_category'],'RuntimeError')
            self.assertEqual(result['prior_result_artifact'],'case-with-1/result.json')

    def test_failed_result_write_retains_actual_worker_protocol_and_usage(self):
        with tempfile.TemporaryDirectory() as name:
            out=Path(name);case={'name':'case','graders':[],'prompt':'synthetic'}
            snapshots={'candidate':{'sha256':'candidate'},'evaluation':{'sha256':'evaluation'}}
            usage={'total':{'totalTokens':12,'inputTokens':10,'outputTokens':2,'cachedInputTokens':0,'reasoningOutputTokens':0}}
            usage['last']=usage['total'].copy();protocol=[{'method':'thread/tokenUsage/updated','usage':usage}]
            client=MagicMock();client.run.return_value={'text':'observed answer','protocol':protocol,'usage':usage,'native_skill_inventory':{'enabled':0}};client.close.return_value=[]
            original=Path.write_text
            def fail_result(path,*args,**kwargs):
                if path.name=='result.json':raise OSError('result write failed')
                return original(path,*args,**kwargs)
            with patch.object(module,'verify_snapshot'),patch.object(module,'CodexClient',return_value=client),patch.object(Path,'write_text',fail_result):
                try:module.evaluate_run(case,'with',1,out,snapshots)
                except OSError as error:result=module.worker_failure_result((case,'with',1),out,snapshots,error)
                else:self.fail('Missing actual write error')
            self.assertEqual(result['protocol'],protocol);self.assertEqual(result['usage'],usage)
            self.assertEqual(result['text'],'observed answer');self.assertEqual(result['error_category'],'OSError')
            client.run.assert_called_once()

    def test_failed_judge_retains_its_partial_protocol_and_usage(self):
        with tempfile.TemporaryDirectory() as name:
            out=Path(name);case={'name':'case','prompt':'synthetic',
                                'graders':[{'name':'strict','type':'llm','rubric':'unchanged'}]}
            snapshots={'candidate':{'sha256':'candidate'},'evaluation':{'sha256':'evaluation'}}
            measured_protocol=[{'usage':{'total':{'totalTokens':100}}}]
            judge_usage={'total':{'totalTokens':17}}
            judge_protocol=[{'usage':judge_usage}]
            client=MagicMock();client.close.return_value=[]
            def run(*args,**kwargs):
                if kwargs.get('judge'):
                    client.last_protocol=judge_protocol
                    raise TimeoutError('actual judge deadline')
                client.last_protocol=measured_protocol
                return {'text':'observed measured answer','trace':[],
                        'protocol':measured_protocol,'usage':{'total':{'totalTokens':100}},'native_skill_inventory':{'enabled':0}}
            client.run.side_effect=run
            with patch.object(module,'verify_snapshot'),patch.object(module,'CodexClient',return_value=client):
                result=module.evaluate_run(case,'with',1,out,snapshots)
            self.assertEqual(result['error_phase'],'judge')
            self.assertEqual(result['protocol'],measured_protocol)
            self.assertEqual(result['error_protocol'],judge_protocol)
            self.assertEqual(result['partial_usage'],judge_usage)
            self.assertEqual(result['partial_usage_scope'],'judge')
            self.assertEqual(result['text'],'observed measured answer')
            self.assertEqual(result['judge_usage'],[])

    def test_malformed_judge_is_an_error_with_usage_and_attempted_denominator(self):
        with tempfile.TemporaryDirectory() as name:
            out=Path(name);snapshots={'candidate':{'sha256':'a'},'evaluation':{'sha256':'b'}}
            case={'name':'bad-judge','runs':1,'prompt':'synthetic','graders':[{'type':'llm','name':'rubric','rubric':'check'}]}
            usage={'total':{'totalTokens':17}}
            observation={key:None for key in ['requested_configuration','observed_thread_configuration',
                'execution_model_telemetry','observation_note','native_skill_inventory','effective_features',
                'mcp_inventory','ambient_document_policy','tool_catalog_attestation','tool_catalog_note','thread_id','turn_id']}
            measured={'text':'actual answer','trace':[],'native_skill_inventory':{'enabled':0}}
            client=MagicMock();client.close.return_value=[];client.last_protocol=[]
            client.run.side_effect=[measured,{**observation,'text':'not JSON','usage':usage}]
            with patch.object(module,'verify_snapshot'),patch.object(module,'CodexClient',return_value=client):
                result=module.evaluate_run(case,'with',1,out,snapshots)
            self.assertEqual(result['error_phase'],'judge')
            self.assertEqual(result['error_category'],'JSONDecodeError')
            self.assertEqual(result['verdicts'],[])
            self.assertEqual(result['judge_usage'],[usage])
            summary=module.report([case],[result],out,time.monotonic())
            self.assertEqual(summary['run_counts'],{'expected':2,'attempted':1,'valid':0,'errors':1})
            self.assertIsNone(summary['cases'][0]['with'])
            self.assertFalse(summary['confirmation_passed'])

    def test_main_contains_worker_exceptions_and_finishes_actual_aggregation(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);(root/'skills').mkdir();(root/'skills/SKILL.md').write_text('synthetic skill')
            (root/'evals/skills').mkdir(parents=True)
            case={'name':'broken','runs':1,'prompt':'synthetic','graders':[],'directory':str(root/'evals/skills')}
            with patch.object(module,'ROOT',root),patch.object(module,'discover_cases',return_value=[case]), \
                 patch.object(module,'evaluate_run',side_effect=TypeError('actual worker boundary failure')):
                status=module.main(['--runs','1'])
            self.assertEqual(status,1)
            reports=list((root/'evals/skills/results').glob('*/aggregate-result.json'))
            self.assertEqual(len(reports),1);summary=json.loads(reports[0].read_text())
            self.assertEqual(summary['errors'],2);self.assertFalse(summary['confirmation_passed'])
            self.assertEqual(len(list(reports[0].parent.glob('*/worker-failure.json'))),2)

    def test_snapshot_hash_commits_to_file_paths_and_bytes(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);(root/'SKILL.md').write_bytes(b'candidate')
            first=module.snapshot_manifest(root)
            expected=hashlib.sha256(b'SKILL.md\0candidate\0').hexdigest()
            self.assertEqual(first['sha256'],expected)
            (root/'SKILL.md').rename(root/'changed.md')
            self.assertNotEqual(module.snapshot_manifest(root)['sha256'],first['sha256'])
            (root/'changed.md').write_bytes(b'new candidate')
            self.assertNotEqual(module.snapshot_manifest(root)['files'][0]['sha256'],first['files'][0]['sha256'])

    def test_clean_snapshot_is_readonly_and_tampering_fails(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);source=root/'source';source.mkdir();(source/'SKILL.md').write_text('candidate')
            (source/'__pycache__').mkdir();(source/'__pycache__/junk.pyc').write_bytes(b'compiled')
            (source/'helper.sh').write_text('#!/bin/sh\n');(source/'helper.sh').chmod(0o755)
            target=root/'snapshot';manifest=module.stage_snapshot(source,target)
            self.assertFalse((target/'__pycache__').exists())
            self.assertEqual((target/'SKILL.md').stat().st_mode & 0o222,0)
            self.assertEqual((target/'helper.sh').stat().st_mode & 0o111,0o111)
            module.verify_snapshot(target,manifest)
            (target/'SKILL.md').chmod(0o644);(target/'SKILL.md').write_text('tampered')
            with self.assertRaises(RuntimeError):module.verify_snapshot(target,manifest)
            (source/'linked').symlink_to(source/'SKILL.md')
            with self.assertRaises(RuntimeError):module.stage_snapshot(source,root/'bad')

    def test_snapshot_case_discovery_does_not_skip_its_results_parent(self):
        with tempfile.TemporaryDirectory() as name:
            case=Path(name)/'results/eval-cases/jev/example';case.mkdir(parents=True)
            (case/'prompt.md').write_text('---\nname: example\nruns: 3\n---\nUser prompt')
            (case/'graders').mkdir();(case/'graders/check.md').write_text('---\ntype: llm\n---\nStrict rubric')
            self.assertEqual(len(module.discover_cases(Path(name)/'results/eval-cases')),1)

    def test_errors_are_separate_from_behavior_means_and_deltas(self):
        with tempfile.TemporaryDirectory() as name:
            case={'name':'case','runs':1}
            results=[{'case':'case','arm':'with','repetition':1,'score':1,'error':None},
                     {'case':'case','arm':'without','repetition':1,'score':0,'error':'budget exceeded'}]
            report=module.report([case],results,Path(name),time.monotonic())
            self.assertEqual(report['errors'],1)
            self.assertIsNone(report['cases'][0]['without'])
            self.assertIsNone(report['cases'][0]['delta'])
            self.assertEqual(report['cases'][0]['error_counts'],{'with':0,'without':1})

    def test_cli_override_literals_are_valid_toml(self):
        value={'skills':[{'path':'/tmp/a path/SKILL.md','enabled':False}],'empty':{},'count':3}
        self.assertEqual(tomllib.loads('value='+module.toml_literal(value))['value'],value)

    def test_subscription_transport_rejects_api_key_auth(self):
        module.require_subscription({'account':{'type':'chatgpt'}})
        for result in [{'account':{'type':'apiKey'}},{'account':None}]:
            with self.assertRaises(RuntimeError):module.require_subscription(result)

    def test_existing_corpus_is_read_without_changing_cases(self):
        training = module.discover_cases(Path('evals/skills'), heldout=False)
        heldout = module.discover_cases(Path('evals/skills'), heldout=True)
        # The added CPU training case extends this corpus; prior blind cases stay unchanged.
        self.assertEqual(len(training), 108)
        self.assertEqual(len(heldout), 29)
        self.assertTrue(all(c['runs'] == 3 for c in training + heldout))
        self.assertEqual(sum('clef' in c['name'] for c in training), 19)
        for case in training + heldout:
            self.assertTrue(case['graders'])
            for grader in case['graders']:
                self.assertIn(grader['type'], {'llm', 'regex', 'tool_used', 'skill_order'})

    def test_publisher_case_stages_the_exact_named_request(self):
        case=next(c for c in module.discover_cases(module.ROOT/'evals/skills')
                  if c['name']=='jev-clef-behaviour-publisher-inputs')
        expected=json.loads(case['prompt'].split('```json\n',1)[1].split('```',1)[0])
        with tempfile.TemporaryDirectory() as name:
            workspace=Path(name);module.stage_fixture(case,workspace)
            request=json.loads((workspace/'compatibility.json').read_text())
        self.assertEqual(request,expected)
        self.assertIs(request['state'],False)
        self.assertEqual(len(request['questions']['risk']['criteria']),26)
        self.assertIsNone(request['questions']['eligible']['criteria']['true'])
        self.assertEqual(request['questions']['spaces']['instructions'],' ')

    def test_tool_evidence_uses_actual_calls_not_answer_mentions(self):
        grader = {'type':'tool_used','tool':'Skill','input_match':r'"skill"\s*:\s*"(?:[\w-]+:)?jev"'}
        self.assertFalse(module.deterministic_grade(grader, {'text':'I used Skill jev','trace':[]})['passed'])
        run = {'text':'','trace':[{'tool':'Skill','arguments':{'skill':'jev'},'success':True}]}
        self.assertTrue(module.deterministic_grade(grader,run)['passed'])
        run['trace'][0]['success'] = False
        self.assertFalse(module.deterministic_grade(grader,run)['passed'])

    def test_secondary_skill_requires_prior_successful_primary(self):
        grader={'type':'skill_order','primary':'jev-pilot','secondary':'jev'}
        def skill(name,success=True):
            return {'tool':'Skill','arguments':{'skill':name},'success':success}
        cases=[([skill('jev-pilot')],True),([skill('jev-pilot'),skill('jev')],True),
               ([skill('jev'),skill('jev-pilot')],False),([skill('jev')],False),
               ([skill('jev-pilot',False),skill('jev')],False),
               ([skill('jev-pilot'),{'tool':'Read','arguments':{'file_path':'references/method.md'}},skill('jev')],True),
               ([],True),([skill('jev',False),skill('jev-pilot')],False),
               ([skill('jev-pilot'),skill('plugin:jev')],True),
               ([{'tool':'Skill','arguments':None,'success':False},skill('jev')],False)]
        for trace,expected in cases:
            with self.subTest(trace=trace):
                self.assertEqual(module.deterministic_grade(grader,{'trace':trace,'text':''})['passed'],expected)

    def test_missing_regex_engine_refuses_before_startup(self):
        with tempfile.TemporaryDirectory() as name, \
             patch.object(module.shutil,'which',side_effect=lambda executable:None if executable=='rg' else '/installed/'+executable), \
             patch.object(module.selectors,'DefaultSelector'), \
             patch.object(module.CodexClient,'request',return_value={'account':{'type':'chatgpt'}}), \
             patch.object(module.os,'killpg'), \
             patch.object(module.subprocess,'Popen') as launch:
            module.EVALUATION_STOP.clear()
            with self.assertRaisesRegex(RuntimeError,'installed rg; no model call started'):
                module.CodexClient(Path(name)/'not-started.log')
            launch.assert_not_called()
            self.assertFalse((Path(name)/'not-started.log').exists())

    def test_skill_order_does_not_drop_the_other_routing_checks(self):
        verdicts=[{'type':'skill_order','passed':True},
                  {'type':'tool_used','tool':'Skill','passed':False},
                  {'type':'tool_used','tool':'Skill','passed':True}]
        self.assertAlmostEqual(module.score_verdicts(verdicts),2/3)

    def test_negative_trigger_and_regex_include_whole_trace(self):
        negative = {'type':'tool_used','tool':'Skill','min':0,'max':0}
        self.assertTrue(module.deterministic_grade(negative, {'text':'','trace':[]})['passed'])
        forbidden = {'type':'tool_used','tool':'Read','min':0,'max':0,'input_match':'secret'}
        self.assertFalse(module.deterministic_grade(forbidden, {'text':'','trace':[{'tool':'Read','arguments':{'file_path':'secret.key'},'success':False}]})['passed'])
        regex = {'type':'regex','target':'trace','match':'not_contains','pattern':'LEAK-CANARY'}
        self.assertFalse(module.deterministic_grade(regex, {'text':'safe','trace':[{'result':'LEAK-CANARY'}]})['passed'])

    def test_with_only_indicator_is_not_a_behavior_score(self):
        score = module.score_verdicts([{'type':'llm','passed':True,'weight':1}, {'type':'tool_used','tool':'Skill','passed':False,'arm':'with-only'}])
        self.assertEqual(score,1)
        self.assertEqual(module.score_verdicts([{'type':'tool_used','tool':'Skill','passed':False}]),0)

    def test_baseline_has_no_skill_catalog_or_tool(self):
        with tempfile.TemporaryDirectory() as name:
            tools=module.ReadOnlyTools(Path(name),Path('skills'),False)
            self.assertNotIn('Skill',[t['name'] for t in tools.specs()])
            self.assertNotIn('jev',tools.instructions())
            result=tools.call('Skill',{'skill':'jev'})
            self.assertFalse(result['success'])

    def test_enabled_native_metadata_is_rejected_before_model_call(self):
        module.assert_no_ambient_skills({'data':[{'cwd':'/tmp','errors':[],'skills':[{'enabled':False,'path':'/disabled/SKILL.md'}]}]},Path('/tmp'),{'/disabled/SKILL.md'})
        with self.assertRaises(RuntimeError):
            module.assert_no_ambient_skills({'data':[{'cwd':'/tmp','errors':[],'skills':[{'enabled':True,'path':'/disabled/SKILL.md'}]}]},Path('/tmp'))
        for result in [{},{'data':[]},{'data':[{}]},{'data':[{'cwd':'/wrong','errors':[],'skills':[]}]}]:
            with self.assertRaises(RuntimeError):module.assert_no_ambient_skills(result,Path('/tmp'))

    def test_events_require_exact_thread_and_turn_attribution(self):
        event={'method':'item/completed','params':{'threadId':'current','turnId':'turn','item':{'id':'answer'}}}
        self.assertTrue(module.belongs_to_turn(event,'current','turn'))
        self.assertFalse(module.belongs_to_turn(event,'old','turn'))
        self.assertFalse(module.belongs_to_turn(event,'current','old-turn'))
        del event['params']['turnId']
        with self.assertRaises(RuntimeError):module.belongs_to_turn(event,'current','turn')
        done={'method':'turn/completed','params':{'threadId':'current','turn':{'id':'turn'}}}
        self.assertTrue(module.belongs_to_turn(done,'current','turn'))

    def test_run_filters_old_turns_and_requires_usage(self):
        usage={'total':{'totalTokens':12,'inputTokens':10,'outputTokens':2,'cachedInputTokens':0,'reasoningOutputTokens':0}}
        usage['last']=usage['total'].copy()
        def replay(include_usage,answer="current answer"):
            client=module.CodexClient.__new__(module.CodexClient);client.notifications=[{'stale':'buffer'}]
            events=[{'method':'item/completed','params':{'threadId':'old','turnId':'old','item':{'type':'agentMessage','text':'stale answer'}}},
                    {'method':'turn/completed','params':{'threadId':'old','turn':{'id':'old','status':'completed'}}},
                    {'method':'item/started','params':{'threadId':'current','turnId':'turn','item':{'type':'agentMessage','id':'answer'}}},
                    {'method':'item/completed','params':{'threadId':'current','turnId':'turn','item':{'type':'agentMessage','id':'answer','text':answer}}}]
            if include_usage:events.append({'method':'thread/tokenUsage/updated','params':{'threadId':'current','turnId':'turn','tokenUsage':usage}})
            events.append({'method':'turn/completed','params':{'threadId':'current','turn':{'id':'turn','status':'completed'}}})
            stream=iter(events);client.receive=lambda deadline:next(stream)
            def request(method,params,timeout=30):
                if method=='thread/start':return {'model':module.MODEL,'reasoningEffort':module.EFFORT,'modelProvider':'openai','approvalPolicy':'never','sandbox':{'type':'readOnly'},'thread':{'id':'current'}}
                if method=='experimentalFeature/list':return {'data':[{'name':name,'enabled':value} for name,value in module.ISOLATION_FEATURES.items()],'nextCursor':None}
                if method=='config/read':return {'config':{'mcp_servers':{},'project_doc_max_bytes':0}}
                if method=='mcpServerStatus/list':return {'data':[],'nextCursor':None}
                if method=='skills/list':return {'data':[{'cwd':'/tmp','errors':[],'skills':[]}]}
                if method=='turn/start':return {'turn':{'id':'turn'}}
                raise AssertionError(method)
            client.request=request
            with patch.object(module,'disabled_ambient_skills',return_value=[]):
                return client.run(Path('/tmp'),'synthetic request',None)
        result=replay(True)
        self.assertEqual(result['text'],'current answer')
        self.assertEqual(result['observed_thread_configuration']['effort'],'high')
        self.assertEqual(result['rounds'],1)
        with self.assertRaises(RuntimeError):replay(False)
        for answer in ['', '  \n', None, 7]:
            with self.subTest(answer=answer),self.assertRaises(RuntimeError):replay(True,answer)

    def test_iterate_is_not_confirmation_and_negative_zero_delta_is_valid(self):
        with tempfile.TemporaryDirectory() as name:
            cases=[{'name':'negative','runs':1}];run={'case':'negative','repetition':1,'score':1,'error':None}
            development=module.report(cases,[{**run,'arm':'with'}],Path(name),time.monotonic(),mode='iterate')
            self.assertFalse(development['incomplete']);self.assertTrue(development['development_only'])
            self.assertFalse(development['confirmation_passed'])
            confirmation=module.report(cases,[{**run,'arm':'with'},{**run,'arm':'without'}],Path(name),time.monotonic())
            self.assertTrue(confirmation['confirmation_passed']);self.assertEqual(confirmation['positive_delta_cases'],0)

    def test_observation_rejects_mismatched_server_configuration(self):
        observed={'model':module.MODEL,'reasoningEffort':module.EFFORT,'modelProvider':'openai','approvalPolicy':'never','sandbox':{'type':'readOnly'},'thread':{'id':'thread'}}
        self.assertEqual(module.observed_configuration(observed)['model'],module.MODEL)
        for key,value in [('model','other'),('reasoningEffort','low'),('modelProvider','custom'),('sandbox',{'type':'dangerFullAccess'})]:
            with self.assertRaises(RuntimeError):module.observed_configuration({**observed,key:value})

    def test_usage_is_required_and_validated(self):
        total={'totalTokens':12,'inputTokens':10,'outputTokens':2,'cachedInputTokens':4,'reasoningOutputTokens':1}
        module.validate_usage({'total':total,'last':total})
        for usage in [None,{}, {'total':total}, {'total':{**total,'outputTokens':-1},'last':total}]:
            with self.assertRaises(RuntimeError):module.validate_usage(usage)

    def test_native_tools_fail_on_start_before_side_effect_completion(self):
        for kind in ['commandExecution','fileChange','webSearch','mcpToolCall','collabAgentToolCall']:
            with self.assertRaises(RuntimeError):module.assert_permitted_item({'type':kind})
        module.assert_permitted_item({'type':'dynamicToolCall'})

    def test_only_bounded_model_and_custom_tool_items_are_permitted(self):
        for kind in ['userMessage','agentMessage','reasoning','dynamicToolCall','functionCallOutput','plan']:
            module.assert_permitted_item({'type':kind})
        for item in [{},None,[],{'type':'unknownFutureItem'},*({'type':kind} for kind in
                     ['hookPrompt','subAgentActivity','imageView','imageGeneration','sleep',
                      'enteredReviewMode','exitedReviewMode','contextCompaction'])]:
            with self.subTest(item=item),self.assertRaises(RuntimeError):module.assert_permitted_item(item)

    def test_ungranted_items_fail_at_both_start_and_completion(self):
        for method in ['item/started','item/completed']:
            for kind in ['commandExecution','fileChange','webSearch','mcpToolCall','collabAgentToolCall',
                         'hookPrompt','subAgentActivity','imageView','imageGeneration','sleep',
                         'enteredReviewMode','exitedReviewMode','contextCompaction','unknownFutureItem']:
                client=module.CodexClient.__new__(module.CodexClient);client.notifications=[]
                events=iter([{'method':method,'params':{'threadId':'current','turnId':'turn',
                             'item':{'type':kind,'id':'ungranted'}}}])
                client.receive=lambda deadline:next(events)
                def request(name,params,timeout=30):
                    if name=='thread/start':return {'model':module.MODEL,'reasoningEffort':module.EFFORT,
                        'modelProvider':'openai','approvalPolicy':'never','sandbox':{'type':'readOnly'},'thread':{'id':'current'}}
                    if name=='experimentalFeature/list':return {'data':[{'name':n,'enabled':v} for n,v in module.ISOLATION_FEATURES.items()],'nextCursor':None}
                    if name=='config/read':return {'config':{'mcp_servers':{},'project_doc_max_bytes':0}}
                    if name=='mcpServerStatus/list':return {'data':[],'nextCursor':None}
                    if name=='skills/list':return {'data':[{'cwd':'/tmp','errors':[],'skills':[]}]}
                    if name=='turn/start':return {'turn':{'id':'turn'}}
                    raise AssertionError(name)
                client.request=request
                with self.subTest(method=method,kind=kind),patch.object(module,'disabled_ambient_skills',return_value=[]),self.assertRaises(RuntimeError):
                    client.run(Path('/tmp'),'synthetic request',None)

    def test_effective_features_must_match_the_grant(self):
        data=[{'name':name,'enabled':value} for name,value in module.ISOLATION_FEATURES.items()]
        module.assert_isolated_features({'data':data,'nextCursor':None})
        for result in [{'data':data[:-1],'nextCursor':None},{'data':data,'nextCursor':'more'},
                       {'data':[{'name':r['name'],'enabled':True} for r in data],'nextCursor':None}]:
            with self.assertRaises(RuntimeError):module.assert_isolated_features(result)

    def test_read_path_and_symlink_cannot_escape(self):
        with tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as outside:
            root=Path(name);secret=Path(outside)/'hidden';secret.write_text('PRIVATE-CANARY')
            (root/'escape').symlink_to(secret)
            tools=module.ReadOnlyTools(root,Path('skills'),False)
            for path in [str(secret),'../hidden','escape']:
                result=tools.call('Read',{'file_path':path})
                self.assertFalse(result['success'])
                self.assertNotIn('PRIVATE-CANARY',json.dumps(result))

    def test_ambient_skill_inventory_follows_directory_symlinks(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);real=root/'real';real.mkdir();(real/'SKILL.md').write_text('test')
            catalog=root/'skills';catalog.mkdir();(catalog/'linked').symlink_to(real,target_is_directory=True)
            paths=module.skill_inventory([catalog])
            self.assertIn(str(real/'SKILL.md'),paths)
            self.assertIn(str(catalog/'linked'/'SKILL.md'),paths)

    def test_ambient_inventory_does_not_follow_arbitrary_external_trees(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);external=root/'external';(external/'nested').mkdir(parents=True)
            (external/'nested/SKILL.md').write_text('private')
            catalog=root/'catalog';catalog.mkdir();(catalog/'everything').symlink_to(external,target_is_directory=True)
            self.assertEqual(module.skill_inventory([catalog]),set())
            with self.assertRaises(RuntimeError):module.skill_inventory([external],maximum_entries=1)

    def test_inference_budget_ignores_duplicate_reasoning_notifications(self):
        budget=module.InferenceBudget(1)
        budget.note_item('reasoning','reasoning-item-a');budget.note_item('reasoning','reasoning-item-a')
        self.assertEqual(budget.rounds,1)
        budget.note_usage({'total':{'totalTokens':100}})
        with self.assertRaises(RuntimeError):budget.note_item('reasoning','reasoning-item-b')

    def test_inference_budget_counts_zero_reasoning_and_batches(self):
        budget=module.InferenceBudget(2)
        budget.note_item('userMessage','user')
        budget.note_item('dynamicToolCall','glob-a');budget.note_item('dynamicToolCall','glob-b')
        self.assertEqual(budget.rounds,1)
        budget.note_usage({'total':{'totalTokens':100}})
        budget.note_item('reasoning','reason-a');budget.note_item('reasoning','reason-b')
        budget.note_usage({'total':{'totalTokens':100}})  # Duplicate must not split the response.
        budget.note_item('dynamicToolCall','read-a');budget.note_item('agentMessage','answer-a')
        self.assertEqual(budget.rounds,2)
        budget.note_usage({'total':{'totalTokens':200}})
        with self.assertRaises(RuntimeError):budget.note_item('agentMessage','third-response')

    def test_judge_malformed_or_disagreeing_votes_fail(self):
        self.assertFalse(module.judge_passed([{'passed':True},{'passed':False},{'passed':False}]))
        with self.assertRaises(ValueError):module.parse_judge('not JSON')
        self.assertEqual(module.parse_judge('{"passed":false,"reason":"missing provider"}')['passed'],False)

    def test_batched_judges_cover_every_rubric_exactly_once(self):
        good='{"verdicts":[{"name":"a","passed":true,"reason":"covered"},{"name":"b","passed":false,"reason":"missing"}]}'
        self.assertEqual(set(module.parse_judge_bundle(good,{'a','b'})),{'a','b'})
        for text in ['{"verdicts":[]}',good.replace('"b"','"a"'),good.replace('"b"','"extra"')]:
            with self.assertRaises(ValueError):module.parse_judge_bundle(text,{'a','b'})

    def test_protocol_requires_exact_model_and_effort(self):
        config=module.thread_params(Path('/tmp'), 'neutral', [], judge=False)
        self.assertEqual(config['model'],'gpt-6.1-sol')
        self.assertEqual(config['config']['model_reasoning_effort'],'high')
        self.assertFalse(config['allowProviderModelFallback'])
        self.assertEqual(config['sandbox'],'read-only')
        self.assertFalse(config['config']['features.shell_tool'])
        self.assertFalse(config['config']['features.apps'])
        self.assertFalse(config['config']['features.plugins'])


if __name__=='__main__':unittest.main()
