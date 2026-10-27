"""Runtime-smoke logic against a stand-in for the product's observable CLI contract.

No many-ai-cli binary or Hub is started here. FakeProduct mimics only what
rust/src/cli.rs, application/hub_status.rs and maintenance_cli/stop.rs print and
write; the process-handling tests start short-lived Python interpreters.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'scripts' / 'rust-candidate-smoke.py'
spec = importlib.util.spec_from_file_location('candidate_smoke', SOURCE)
smoke = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = smoke  # dataclasses resolve their module while the class is created
spec.loader.exec_module(smoke)

VERSION = '0.0.0-rust-candidate+abcdef123456'
COMMIT = 'a' * 40
BUILD_TIME = '2026-10-27T00:00:00+00:00'
TARGET = 'x86_64-unknown-linux-gnu'
TOKEN = 'ab12cd34' * 8  # 64 hex, the shape random_token() produces
PORT = 49123


class FakeClock:
    def __init__(self):
        self.value = 1000.0

    def now(self):
        return self.value

    def sleep(self, seconds):
        self.value += seconds


class FakeProduct:
    """State of the stand-in Hub plus the knobs for each failure scenario."""

    def __init__(self, clock, **scenario):
        self.clock = clock
        self.scenario = scenario
        self.running = False
        self.exit_code = None
        self.status_polls_left = scenario.get('stopped_polls_before_ready', 2)
        self.pid = scenario.get('pid', 4242)
        self.commands = []
        self.environments = []
        self.cwds = []
        self.signals = []
        self.closed = False

    def option(self, name, default=None):
        return self.scenario.get(name, default)


class FakeHandle:
    def __init__(self, product):
        self.product = product
        self.pid = product.pid

    def poll(self):
        return None if self.product.running else self.product.exit_code

    def wait(self, timeout):
        if self.product.running:
            self.product.clock.sleep(timeout)
            return None
        return self.product.exit_code

    def terminate(self):
        self.product.signals.append('terminate')
        if not self.product.option('ignore_terminate'):
            self.product.running, self.product.exit_code = False, -15

    def kill(self):
        self.product.signals.append('kill')
        self.product.running, self.product.exit_code = False, -9

    def close(self):
        self.product.closed = True


class FakeExecutor:
    def __init__(self, product):
        self.product = product

    def run(self, argv, env, cwd, timeout):
        product = self.product
        assert argv[1] == '--trial-root' and argv[3] == '--trial-port', argv
        state, port, tail = Path(argv[2]), int(argv[4]), tuple(argv[5:])
        product.commands.append(tail)
        product.environments.append(env)
        product.cwds.append(cwd)
        if product.option('exec_error') and tail == ('version',):
            raise PermissionError('synthetic')
        if tail == ('version',):
            return smoke.CommandResult(0, (product.option('version', VERSION) + '\n').encode())
        # MainContext::load creates the private config (and its token) on first use.
        config = state / 'config.yaml'
        if not config.exists():
            config.write_text(f'token: {TOKEN}\nhub:\n  port: {port}\n')
        if tail == ('status',):
            if product.running and product.status_polls_left > 0:
                product.status_polls_left -= 1
            elif product.running and not product.option('never_ready'):
                text = f'running http://127.0.0.1:{port}/?token={TOKEN}\n'
                if product.option('stale_warning'):
                    text += 'WARNING: running Hub is a STALE binary (the on-disk exe differs).\n'
                return smoke.CommandResult(0, text.encode())
            return smoke.CommandResult(0, b'stopped\n')
        if tail == ():
            if not product.running:
                return smoke.CommandResult(None, timed_out=True)  # it would have become a foreground Hub
            if product.option('replace_hub_on_reuse'):
                product.pid += 1
                (state / 'hub-runtime.json').write_text(json.dumps({'version': 1, 'pid': product.pid, 'port': port}))
            return smoke.CommandResult(0)
        if tail == ('stop',):
            if not product.running or product.option('stop_fails'):
                return smoke.CommandResult(1, b'', f'hub pid not found token={TOKEN}'.encode())
            if not product.option('stop_does_not_end_hub'):
                product.running = False
                product.exit_code = product.option('serve_exit_code', 0)
                if not product.option('ledger_remains'):
                    (state / 'hub-runtime.json').unlink(missing_ok=True)
            return smoke.CommandResult(0)
        raise AssertionError(f'unexpected command: {tail}')

    def spawn(self, argv, env, cwd, stderr_path):
        product = self.product
        assert argv[5:] == ['serve', '--open=false', '--strict-port'], argv
        product.commands.append(('serve',))
        product.environments.append(env)
        product.cwds.append(cwd)
        state = Path(argv[2])
        stderr_path.write_text(product.option('serve_stderr', ''))
        if product.option('spawn_error'):
            raise FileNotFoundError('synthetic')
        if product.option('exit_early'):
            product.exit_code = 1
            return FakeHandle(product)
        product.running = True
        if not product.option('no_ledger'):
            (state / 'hub-runtime.json').write_text(json.dumps({
                'version': 1, 'pid': product.pid + product.option('ledger_pid_offset', 0), 'port': int(argv[4]),
                'started_at': '2026-10-27T00:00:00Z'}))
        return FakeHandle(product)


class SmokeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.parent = Path(self.temp.name)
        self.binary = self.parent / 'many-ai-cli'
        self.binary.write_bytes(b'synthetic binary bytes')
        self.sha = hashlib.sha256(self.binary.read_bytes()).hexdigest()

    def attempt(self, *, info=None, total_seconds=smoke.TOTAL_SECONDS, port=PORT, wrap=None, **scenario):
        clock = FakeClock()
        product = FakeProduct(clock, **scenario)
        self.logs = []
        config = smoke.SmokeConfig(
            target=TARGET, binary=self.binary, binary_sha256=self.sha, expected_version=VERSION,
            expected_commit=COMMIT, expected_build_time=BUILD_TIME, temp_parent=self.parent,
            total_seconds=total_seconds)

        def fetch_info(requested_port, token):
            self.assertEqual((requested_port, token), (PORT, TOKEN))
            value = {'version': VERSION, 'git_commit': COMMIT, 'build_time': BUILD_TIME,
                     'binary_sha256': self.sha, 'runtime_mode': 'linux', 'binary_stale': False,
                     'web_dist_fresh': True, 'web_src_hash': 'd6a6c30a5375', 'active_sessions': 0}
            value.update(info or {})
            return value

        executor = FakeExecutor(product)
        runner = smoke.Smoke(config, wrap(executor) if wrap else executor, clock, fetch_info=fetch_info,
                             pick_port=lambda: port, port_open=lambda _: product.running,
                             log=self.logs.append)
        record = runner.run()
        self.product, self.runner = product, runner
        return record

    def statuses(self, record):
        return {check['id']: check['status'] for check in record['checks']}

    def assertNoSecret(self, record):
        everything = json.dumps(record) + '\n'.join(self.logs)
        self.assertNotIn(TOKEN, everything)
        self.assertNotIn(TOKEN[:20], everything)

    def assertCleanedUp(self, record, *, forced=False):
        self.assertTrue(record['cleanup']['hub_stopped'])
        self.assertTrue(record['cleanup']['root_removed'])
        self.assertEqual(record['cleanup']['forced_termination'], forced)
        self.assertFalse(self.runner.root.exists())
        self.assertEqual([p.name for p in self.parent.iterdir()], ['many-ai-cli'])

    def test_full_cycle_passes_and_records_every_check_without_the_token(self):
        record = self.attempt()
        self.assertEqual(record['status'], 'passed')
        self.assertIsNone(record['failure'])
        self.assertEqual(list(self.statuses(record)), list(smoke.CHECK_IDS))
        self.assertEqual(set(self.statuses(record).values()), {'passed'})
        self.assertEqual(self.product.commands, [
            ('version',), ('status',), ('serve',), ('status',), ('status',), ('status',), ('status',),
            (), ('status',), ('stop',), ('status',)])
        self.assertEqual(record['port'], PORT)
        self.assertEqual(record['serve'], {'pid': 4242, 'exit_code': 0})
        self.assertEqual(record['isolation']['home_after'], {'files': 0, 'directories': 0})
        self.assertFalse(record['cleanup']['graceful_stop_attempted'])
        self.assertCleanedUp(record)
        self.assertNoSecret(record)
        self.assertTrue(self.product.closed)

    HUB_LOG = (
        'time=2026-10-27T09:00:00+09:00 level=INFO msg="MANY-AI-CLI started" detail="pid=4242 port=49123"\n'
        'time=2026-10-27T09:00:00+09:00 level=WARN msg="session history disabled" detail="unavailable"\n'
        'time=2026-10-27T09:00:01+09:00 level=INFO msg="MANY-AI-CLI stopping" detail="reason=cli_stop"\n')

    def test_a_passing_receipt_keeps_hub_log_counts_but_no_log_text(self):
        record = self.attempt(serve_stderr=self.HUB_LOG)
        self.assertEqual(record['status'], 'passed')
        self.assertEqual(record['serve']['stderr'], {'lines': 3, 'levels': {'INFO': 2, 'WARN': 1},
                                                      'problems': ['"session history disabled"']})
        self.assertNotIn('stderr_tail', record['serve'])
        self.assertNotIn('pid=4242', json.dumps(record))

    def test_a_failing_receipt_keeps_a_redacted_hub_log_tail(self):
        record = self.attempt(stale_warning=True, serve_stderr=self.HUB_LOG + f'time=x level=ERROR msg="boom token={TOKEN}"\n')
        self.assertEqual(record['status'], 'failed')
        self.assertIn('session history disabled', record['serve']['stderr_tail'])
        self.assertEqual(record['serve']['stderr']['levels']['ERROR'], 1)
        self.assertNoSecret(record)

    def test_every_command_runs_in_the_trial_root_with_an_isolated_environment(self):
        self.attempt()
        root = self.runner.root  # removed by now; compare against the recorded paths
        for cwd in self.product.cwds:
            self.assertEqual(cwd.name, 'work')
            self.assertEqual(cwd.parent.name, 'state')
        for env in self.product.environments:
            self.assertEqual(env['MANY_AI_CLI_HOST_LABEL'], 'synthetic-isolated')
            for key in ('HOME', 'USERPROFILE', 'TMPDIR', 'XDG_CONFIG_HOME', 'CODEX_HOME'):
                self.assertTrue(env[key].startswith(str(root)), key)

    def test_version_mismatch_stops_before_anything_is_started(self):
        record = self.attempt(version='0.0.0-other')
        self.assertEqual(record['failure'], {'check': 'version', 'code': 'version_mismatch'})
        self.assertEqual(self.statuses(record)['version'], 'failed')
        self.assertEqual(self.statuses(record)['serve-start'], 'not-run')
        self.assertEqual(self.product.commands, [('version',)])
        self.assertFalse(record['cleanup']['hub_started'])
        self.assertEqual(record['status'], 'failed')
        self.assertCleanedUp(record)

    def test_binary_that_cannot_be_executed_is_reported_and_cleaned_up(self):
        record = self.attempt(exec_error=True)
        self.assertEqual(record['failure'], {'check': 'version', 'code': 'exec_failed'})
        self.assertCleanedUp(record)

    def test_wrong_artifact_hash_is_refused_before_a_directory_is_created(self):
        self.sha = '0' * 64
        record = self.attempt()
        self.assertEqual(record['failure'], {'check': 'prepare', 'code': 'binary_hash_mismatch'})
        self.assertIsNone(self.runner.root)
        self.assertEqual([p.name for p in self.parent.iterdir()], ['many-ai-cli'])

    def test_default_port_is_refused(self):
        record = self.attempt(port=47777)
        self.assertEqual(record['failure'], {'check': 'prepare', 'code': 'port_refused'})
        self.assertCleanedUp(record)

    def test_serve_that_exits_early_reports_redacted_stderr(self):
        record = self.attempt(exit_early=True, serve_stderr=f'bind failed http://127.0.0.1:1/?token={TOKEN}\n')
        self.assertEqual(record['failure'], {'check': 'serve-start', 'code': 'serve_exited_early'})
        detail = record['checks'][3]['detail']
        self.assertEqual(detail['exit_code'], 1)
        self.assertIn('bind failed', detail['stderr_tail'])
        self.assertIn('<redacted>', detail['stderr_tail'])
        self.assertNoSecret(record)
        self.assertCleanedUp(record)

    def test_hub_that_never_answers_times_out_and_is_force_stopped(self):
        record = self.attempt(never_ready=True, ignore_terminate=True, stop_does_not_end_hub=True)
        self.assertEqual(record['failure'], {'check': 'serve-start', 'code': 'serve_not_ready'})
        self.assertGreater(record['checks'][3]['detail']['status_polls'], 10)
        self.assertLess(record['total_seconds'], smoke.START_SECONDS + 60)
        self.assertEqual(self.product.signals, ['terminate', 'kill'])
        self.assertTrue(record['cleanup']['graceful_stop_attempted'])
        self.assertCleanedUp(record, forced=True)
        self.assertEqual(record['serve']['exit_code'], -9)
        self.assertNoSecret(record)

    def test_stale_binary_warning_fails_the_status_check(self):
        record = self.attempt(stale_warning=True)
        self.assertEqual(record['failure'], {'check': 'status-running', 'code': 'stale_binary_warning'})
        self.assertCleanedUp(record)

    def test_ledger_must_describe_the_process_that_was_started(self):
        record = self.attempt(ledger_pid_offset=1)
        self.assertEqual(record['failure'], {'check': 'ledger', 'code': 'ledger_mismatch'})
        self.assertFalse(record['checks'][4]['detail']['pid_matches'])
        record = self.attempt(no_ledger=True)
        self.assertEqual(record['failure'], {'check': 'ledger', 'code': 'ledger_unreadable'})
        self.assertCleanedUp(record)

    def test_info_must_carry_the_build_stamp_and_a_fresh_binary(self):
        for override in ({'version': 'x'}, {'git_commit': 'b' * 40}, {'build_time': 'x'},
                         {'binary_sha256': '1' * 64}, {'runtime_mode': 'darwin'},
                         {'binary_stale': True}, {'web_dist_fresh': False}, {'version': None}):
            with self.subTest(override=override):
                record = self.attempt(info=override)
                self.assertEqual(record['failure'], {'check': 'api-info', 'code': 'info_mismatch'})
                self.assertEqual(record['checks'][6]['detail']['fields'], list(override))
                self.assertNoSecret(record)
                self.assertCleanedUp(record)

    def test_default_command_must_reuse_the_running_hub(self):
        record = self.attempt(replace_hub_on_reuse=True)
        self.assertEqual(record['failure'], {'check': 'reuse-existing-hub', 'code': 'hub_changed_by_default_command'})
        self.assertCleanedUp(record)

    def test_failing_stop_leaves_the_hub_to_the_cleanup_which_still_removes_it(self):
        record = self.attempt(stop_fails=True)
        self.assertEqual(record['failure'], {'check': 'stop', 'code': 'stop_exit_nonzero'})
        detail = record['checks'][8]['detail']
        self.assertNotIn(TOKEN, json.dumps(detail))
        self.assertTrue(record['cleanup']['graceful_stop_attempted'])
        self.assertEqual(record['cleanup']['forced_termination'], True)
        self.assertEqual(self.product.signals, ['terminate'])
        self.assertCleanedUp(record, forced=True)

    def test_hub_that_ignores_stop_fails_the_exit_wait_and_is_forced_down(self):
        record = self.attempt(stop_does_not_end_hub=True)
        self.assertEqual(record['failure'], {'check': 'stop', 'code': 'serve_still_running_after_stop'})
        self.assertCleanedUp(record, forced=True)

    def test_non_zero_serve_exit_code_after_stop_fails(self):
        record = self.attempt(serve_exit_code=101)
        self.assertEqual(record['failure'], {'check': 'stop', 'code': 'serve_exit_nonzero'})
        self.assertEqual(record['serve']['exit_code'], 101)
        self.assertCleanedUp(record)

    def test_ledger_left_behind_after_stop_fails(self):
        record = self.attempt(ledger_remains=True)
        self.assertEqual(record['failure'], {'check': 'stopped-after', 'code': 'ledger_remains'})
        self.assertCleanedUp(record)

    def test_total_deadline_ends_the_smoke_instead_of_waiting_for_the_job_limit(self):
        record = self.attempt(never_ready=True, stop_does_not_end_hub=True, total_seconds=5.0)
        self.assertEqual(record['failure']['check'], 'serve-start')
        self.assertIn(record['failure']['code'], ('serve_not_ready', 'deadline_exceeded'))
        self.assertLess(record['total_seconds'], 60)
        self.assertCleanedUp(record, forced=True)

    def test_interrupt_still_cleans_up_and_is_recorded(self):
        class InterruptFirstStop:
            """Like a SIGTERM from the caller arriving while `stop` runs."""
            def __init__(self, executor):
                self.executor, self.fired = executor, False

            def run(self, argv, env, cwd, timeout):
                if argv[5:] == ['stop'] and not self.fired:
                    self.fired = True
                    raise KeyboardInterrupt
                return self.executor.run(argv, env, cwd, timeout)

            def spawn(self, *args):
                return self.executor.spawn(*args)
        record = self.attempt(wrap=InterruptFirstStop)
        self.assertEqual(record['failure'], {'check': 'run', 'code': 'interrupted', 'error': 'KeyboardInterrupt'})
        self.assertTrue(record['cleanup']['graceful_stop_attempted'])
        self.assertEqual(self.statuses(record)['stop'], 'not-run')
        self.assertCleanedUp(record)

    def test_second_interrupt_during_cleanup_does_not_leave_the_hub(self):
        class InterruptEveryStop:
            def __init__(self, executor):
                self.executor = executor

            def run(self, argv, env, cwd, timeout):
                if argv[5:] == ['stop']:
                    raise KeyboardInterrupt
                return self.executor.run(argv, env, cwd, timeout)

            def spawn(self, *args):
                return self.executor.spawn(*args)
        record = self.attempt(wrap=InterruptEveryStop)
        self.assertEqual(record['failure']['code'], 'interrupted')
        self.assertEqual(self.product.signals, ['terminate'])
        self.assertCleanedUp(record, forced=True)


class PureHelperTests(unittest.TestCase):
    def test_redaction_removes_known_and_token_shaped_values(self):
        text = f'GET /?token={TOKEN}&x=1 Authorization: Bearer {TOKEN} other {"f" * 40} short abcdef12 secret-xyz'
        cleaned = smoke.redact(text, ('secret-xyz',))
        for leaked in (TOKEN, 'f' * 40, 'secret-xyz'):
            self.assertNotIn(leaked, cleaned)
        self.assertIn('short abcdef12', cleaned)
        self.assertEqual(smoke.redact('token=abc'), 'token=<redacted>')

    def test_tail_redacts_before_cutting_so_a_cut_token_cannot_survive(self):
        data = (b'x' * 1190) + TOKEN.encode() + b' end'
        tail = smoke.tail_text(data, limit=1200)
        self.assertNotIn(TOKEN[:10], tail)
        self.assertTrue(tail.endswith('<redacted> end'))

    def test_status_parsing_keeps_only_the_endpoint_and_flags(self):
        running = smoke.parse_status(f'running http://127.0.0.1:{PORT}/?token={TOKEN}\n'.encode())
        self.assertEqual(running, {'kind': 'running', 'scheme': 'http', 'host': '127.0.0.1', 'port': PORT,
                                   'stale_warning': False, 'extra_lines': 0})
        self.assertNotIn(TOKEN, json.dumps(running))
        stale = smoke.parse_status(b'running http://127.0.0.1:1/?token=a\nWARNING: x\n  restart\n')
        self.assertTrue(stale['stale_warning'])
        self.assertEqual(smoke.parse_status(b'stopped\r\n'), {'kind': 'stopped'})
        self.assertEqual(smoke.parse_status(b'stopped\nextra\n')['kind'], 'other')
        self.assertEqual(smoke.parse_status(b'')['kind'], 'other')
        self.assertEqual(smoke.parse_status(b'running http://[bad/\n')['port'], None)

    def test_token_is_read_from_plain_or_quoted_yaml_and_never_guessed(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'config.yaml'
            for text in (f'token: {TOKEN}\n', f'hub:\n  port: 1\ntoken: "{TOKEN}"\r\nx: 1\n',
                         f"token: '{TOKEN}' \n"):
                path.write_text(text, newline='')
                self.assertEqual(smoke.read_token(path), TOKEN)
            for text in ('hub:\n  token: abc\n', 'token: short\n', 'token:\n', ''):
                path.write_text(text)
                self.assertIsNone(smoke.read_token(path))
            self.assertIsNone(smoke.read_token(Path(temp) / 'missing.yaml'))

    def test_environment_is_built_from_scratch_inside_the_root(self):
        host = {'PATH': '/runner/bin', 'GITHUB_TOKEN': 'x', 'ACTIONS_RUNTIME_TOKEN': 'y', 'HOME': '/real/home',
                'SystemRoot': r'C:\Windows'}
        root = Path(tempfile.gettempdir()) / 'synthetic-root'
        for windows in (False, True):
            with self.subTest(windows=windows):
                env = smoke.isolated_environment(root, windows=windows, host=host)
                self.assertNotIn('GITHUB_TOKEN', env)
                self.assertNotIn('ACTIONS_RUNTIME_TOKEN', env)
                self.assertNotIn('/runner/bin', env['PATH'])
                self.assertNotIn('/real/home', json.dumps(env))
                self.assertEqual(env['HOME'], str(root / 'home'))
                self.assertEqual(env['USERPROFILE'], str(root / 'home'))
                for key in ('TMPDIR', 'TEMP', 'TMP', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME',
                            *smoke.PROFILE_VARIABLES):
                    self.assertTrue(Path(env[key]).is_relative_to(root), key)
        self.assertEqual(smoke.isolated_environment(root, windows=False, host=host)['PATH'], '/usr/bin:/bin:/usr/sbin:/sbin')
        windows_env = smoke.isolated_environment(root, windows=True, host=host)
        self.assertTrue(Path(windows_env['APPDATA']).is_relative_to(root))
        self.assertTrue(Path(windows_env['LOCALAPPDATA']).is_relative_to(root))
        self.assertEqual(windows_env['SystemRoot'], r'C:\Windows')

    def test_runtime_mode_follows_the_target(self):
        self.assertEqual(smoke.expected_runtime_mode('x86_64-pc-windows-msvc'), 'windows-native')
        self.assertEqual(smoke.expected_runtime_mode('x86_64-unknown-linux-gnu'), 'linux')
        self.assertEqual(smoke.expected_runtime_mode('aarch64-apple-darwin'), 'darwin')
        self.assertEqual(smoke.expected_runtime_mode('x86_64-apple-darwin'), 'darwin')
        with self.assertRaises(ValueError):
            smoke.expected_runtime_mode('riscv64-unknown-linux-gnu')

    def test_remove_tree_deletes_read_only_content_and_reports_absence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'tree'
            (root / 'a' / 'b').mkdir(parents=True)
            locked = root / 'a' / 'b' / 'file'
            locked.write_text('x')
            os.chmod(locked, 0o444)
            self.assertTrue(smoke.remove_tree(root, pause=0))
            self.assertFalse(root.exists())
            self.assertTrue(smoke.remove_tree(root, pause=0))


class RealProcessTests(unittest.TestCase):
    """Process handling with short-lived Python interpreters (never the product)."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.cwd = Path(self.temp.name)
        self.env = os.environ.copy()
        self.executor = smoke.SubprocessExecutor()

    def test_run_captures_streams_and_kills_a_command_that_hangs(self):
        done = self.executor.run([sys.executable, '-c', 'import sys;print("out");print("err",file=sys.stderr);sys.exit(3)'],
                                 self.env, self.cwd, 30)
        self.assertEqual((done.code, done.timed_out), (3, False))
        self.assertIn(b'out', done.stdout)
        self.assertIn(b'err', done.stderr)
        started = time.monotonic()
        hung = self.executor.run([sys.executable, '-c', 'import time;time.sleep(60)'], self.env, self.cwd, 1.0)
        self.assertTrue(hung.timed_out)
        self.assertIsNone(hung.code)
        self.assertLess(time.monotonic() - started, 30)

    def test_spawned_process_is_reaped_when_it_exits_and_can_be_terminated(self):
        quick = self.executor.spawn([sys.executable, '-c', 'raise SystemExit(5)'], self.env, self.cwd,
                                    self.cwd / 'quick.log')
        self.assertEqual(quick.wait(30), 5)
        self.assertEqual(quick.poll(), 5)
        quick.close()
        slow = self.executor.spawn([sys.executable, '-c', 'import time;time.sleep(60)'], self.env, self.cwd,
                                   self.cwd / 'slow.log')
        self.assertIsNone(slow.poll())
        self.assertIsNone(slow.wait(0.2))
        slow.terminate()
        self.assertIsNotNone(slow.wait(30))
        slow.close()

    @unittest.skipIf(os.name == 'nt', 'Windows terminate() is already a hard kill')
    def test_kill_follows_a_terminate_that_is_ignored(self):
        log = self.cwd / 'stubborn.log'
        code = ('import signal,sys,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);'
                'print("ready",file=sys.stderr,flush=True);time.sleep(60)')
        handle = self.executor.spawn([sys.executable, '-c', code], self.env, self.cwd, log)
        deadline = time.monotonic() + 30
        while 'ready' not in log.read_text() and time.monotonic() < deadline:
            time.sleep(0.05)
        handle.terminate()
        self.assertIsNone(handle.wait(1.0))
        handle.kill()
        self.assertIsNotNone(handle.wait(30))
        handle.close()

    def test_main_reports_a_binary_that_cannot_run_and_leaves_no_directory(self):
        if os.name != 'nt':
            self.addCleanup(signal.signal, signal.SIGTERM, signal.getsignal(signal.SIGTERM))
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / 'many-ai-cli'
            binary.write_bytes(b'not an executable program')
            result = Path(temp) / 'result.json'
            scratch = Path(temp) / 'scratch'
            scratch.mkdir()
            with patch.object(tempfile, 'tempdir', str(scratch)):
                code = smoke.main(['--target', TARGET, '--binary', str(binary),
                                   '--binary-sha256', hashlib.sha256(binary.read_bytes()).hexdigest(),
                                   '--expected-version', VERSION, '--expected-commit', COMMIT,
                                   '--expected-build-time', BUILD_TIME, '--result', str(result)])
            record = json.loads(result.read_text())
            self.assertEqual(code, 1)
            self.assertEqual(record['status'], 'failed')
            self.assertEqual(record['failure'], {'check': 'version', 'code': 'exec_failed'})
            self.assertEqual([c['id'] for c in record['checks']], list(smoke.CHECK_IDS))
            self.assertEqual(list(scratch.iterdir()), [])


# A stand-in with the same observable contract as the product (cli.rs, hub_status.rs,
# stop.rs): version/status/stop commands, a ledger file, a loopback server that wants the
# token, and a `stop` that decides the Hub is gone with kill(pid, 0) like the real one.
STAND_IN = r"""
import hashlib, http.client, http.server, json, os, secrets, sys, threading, time, urllib.parse

VERSION, COMMIT, BUILD_TIME = '__VERSION__', '__COMMIT__', '__BUILD_TIME__'
IGNORE_SHUTDOWN, HANG_START, EXIT_CODE = __IGNORE_SHUTDOWN__, __HANG_START__, __EXIT_CODE__
args = sys.argv[1:]
assert args[0] == '--trial-root' and args[2] == '--trial-port', args
root, port, tail = args[1], int(args[3]), args[4:]
ledger = os.path.join(root, 'hub-runtime.json')
config = os.path.join(root, 'config.yaml')


def log(level, message, detail=''):
    sys.stderr.write(f'time=2026-10-27T00:00:00+00:00 level={level} msg="{message}" detail="{detail}"\n')
    sys.stderr.flush()


def token():
    if not os.path.exists(config):
        with open(config, 'w') as handle:
            handle.write('token: ' + secrets.token_hex(32) + '\nhub:\n  port: %d\n' % port)
    for line in open(config).read().splitlines():
        if line.startswith('token: '):
            return line[7:].strip()


def alive(pid):
    try:
        os.kill(pid, 0)  # true for an exited child nobody has waited for, as in the product
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def request(method, path, secret, bearer=False):
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=2)
    try:
        headers = {'Authorization': 'Bearer ' + secret} if bearer else {}
        connection.request(method, path if bearer else path + '?token=' + secret, headers=headers)
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def ledger_pid():
    try:
        return json.load(open(ledger))['pid']
    except (OSError, ValueError, KeyError):
        return None


def running():
    pid = ledger_pid()
    if pid is None or not alive(pid):
        return False
    try:
        return request('GET', '/', token())[0] == 200
    except OSError:
        return False


def serve():
    secret = token()
    if HANG_START:
        time.sleep(600)
    me = os.path.realpath(sys.argv[0])

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *ignored):
            pass

        def allowed(self):
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
            return query.get('token') == [secret] or self.headers.get('Authorization') == 'Bearer ' + secret

        def reply(self, status, body=b''):
            self.send_response(status)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            path = urllib.parse.urlsplit(self.path).path
            if not self.allowed():
                return self.reply(401)
            if path == '/':
                return self.reply(200, b'<html></html>')
            if path == '/api/info':
                return self.reply(200, json.dumps({
                    'version': VERSION, 'git_commit': COMMIT, 'build_time': BUILD_TIME,
                    'binary_sha256': hashlib.sha256(open(me, 'rb').read()).hexdigest(),
                    'binary_stale': False, 'web_dist_fresh': True, 'web_src_hash': 'd6a6c30a5375',
                    'active_sessions': 0, 'runtime_mode': 'linux'}).encode())
            return self.reply(404)

        def do_POST(self):
            if urllib.parse.urlsplit(self.path).path != '/api/shutdown' or not self.allowed():
                return self.reply(404)
            self.reply(200)
            if not IGNORE_SHUTDOWN:
                threading.Thread(target=server.shutdown, daemon=True).start()

    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler)
    with open(ledger, 'w') as handle:
        json.dump({'version': 1, 'pid': os.getpid(), 'port': port, 'started_at': '2026-10-27T00:00:00Z'}, handle)
    log('INFO', 'MANY-AI-CLI started', 'pid=%d port=%d' % (os.getpid(), port))
    server.serve_forever()
    log('INFO', 'MANY-AI-CLI stopping', 'reason=cli_stop')
    sys.exit(EXIT_CODE)


if tail == ['version']:
    print(VERSION)
elif tail == ['status']:
    print(('running http://127.0.0.1:%d/?token=%s' % (port, token())) if running() else 'stopped')
elif tail[:1] == ['serve']:
    serve()
elif tail == []:
    if not running():
        serve()
elif tail == ['stop']:
    pid = ledger_pid()
    if pid is None:
        sys.stderr.write('hub pid not found\n')
        sys.exit(1)
    try:
        request('POST', '/api/shutdown', token())
    except OSError:
        pass
    deadline = time.time() + 2
    while time.time() < deadline:
        if not alive(pid):
            os.remove(ledger)
            sys.exit(0)
        time.sleep(0.1)
    sys.stderr.write('hub runtime owner changed during stop\n')
    sys.exit(1)
else:
    sys.stderr.write('unexpected command\n')
    sys.exit(2)
"""


@unittest.skipIf(os.name == 'nt', 'the stand-in is a script; Windows needs an .exe to execute')
class StandInProcessTests(unittest.TestCase):
    """The real executor, polling, HTTP, ledger and cleanup code against real processes."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.parent = Path(self.temp.name)

    def attempt(self, *, ignore_shutdown=False, hang_start=False, exit_code=0, start_seconds=20.0):
        text = (STAND_IN.replace('__VERSION__', VERSION).replace('__COMMIT__', COMMIT)
                .replace('__BUILD_TIME__', BUILD_TIME).replace('__IGNORE_SHUTDOWN__', repr(ignore_shutdown))
                .replace('__HANG_START__', repr(hang_start)).replace('__EXIT_CODE__', repr(exit_code)))
        binary = self.parent / 'many-ai-cli'
        binary.write_text(f'#!{sys.executable}\n' + text, encoding='utf-8')
        os.chmod(binary, 0o755)
        config = smoke.SmokeConfig(
            target=TARGET, binary=binary, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
            expected_version=VERSION, expected_commit=COMMIT, expected_build_time=BUILD_TIME,
            temp_parent=self.parent, total_seconds=120.0)
        self.logs = []
        runner = smoke.Smoke(config, smoke.SubprocessExecutor(), smoke.Clock(), log=self.logs.append)
        patches = [patch.object(smoke, 'START_SECONDS', start_seconds),
                   patch.object(smoke, 'CLEANUP_SIGNAL_SECONDS', 1.0)]
        for entry in patches:
            entry.start()
            self.addCleanup(entry.stop)
        self.runner = runner
        return runner.run()

    def test_full_cycle_with_real_processes_sockets_and_files(self):
        # Also the regression test for ProcessHandle's reaper: with `_reap` disabled, the stand-in's
        # kill(pid, 0) based stop sees our exited, unwaited Hub as alive and fails ("owner changed").
        record = self.attempt()
        self.assertEqual(record['failure'], None, record)
        self.assertEqual(record['status'], 'passed')
        self.assertEqual({c['status'] for c in record['checks']}, {'passed'})
        self.assertEqual(record['serve']['exit_code'], 0)
        self.assertEqual(record['serve']['stderr']['levels'], {'INFO': 2})
        self.assertFalse(record['cleanup']['graceful_stop_attempted'])
        self.assertTrue(record['cleanup']['root_removed'])
        self.assertFalse(self.runner.root.exists())
        shipped = json.dumps({key: value for key, value in record.items() if key != 'binary'})
        self.assertIsNone(re.search(r'[0-9a-f]{64}', shipped))  # nothing token-shaped
        self.assertEqual([p.name for p in self.parent.iterdir()], ['many-ai-cli'])

    def test_hub_that_ignores_the_shutdown_request_is_found_and_terminated(self):
        record = self.attempt(ignore_shutdown=True)
        self.assertEqual(record['failure'], {'check': 'stop', 'code': 'stop_exit_nonzero'})
        self.assertTrue(record['cleanup']['graceful_stop_attempted'])
        self.assertTrue(record['cleanup']['forced_termination'])
        self.assertTrue(record['cleanup']['hub_stopped'])
        self.assertLess(record['serve']['exit_code'], 0)
        self.assertTrue(record['cleanup']['root_removed'])

    def test_hub_that_never_binds_times_out_and_is_terminated(self):
        record = self.attempt(hang_start=True, start_seconds=3.0)
        self.assertEqual(record['failure'], {'check': 'serve-start', 'code': 'serve_not_ready'})
        self.assertTrue(record['cleanup']['hub_stopped'])
        self.assertTrue(record['cleanup']['forced_termination'])
        self.assertTrue(record['cleanup']['root_removed'])
        self.assertLess(record['total_seconds'], 60)

    def test_nonzero_exit_of_the_hub_after_a_clean_stop_is_reported(self):
        record = self.attempt(exit_code=3)
        self.assertEqual(record['failure'], {'check': 'stop', 'code': 'serve_exit_nonzero'})
        self.assertEqual(record['serve']['exit_code'], 3)
        self.assertTrue(record['cleanup']['root_removed'])


if __name__ == '__main__':
    unittest.main()
