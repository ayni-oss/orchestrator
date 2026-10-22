#!/usr/bin/env python3
"""Generate the bounded Git snapshot used by the task acceptance worker."""
import hashlib,json,os,pathlib,subprocess,tempfile
root=pathlib.Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix='ayni-task-source-') as temporary:
    checkout=pathlib.Path(temporary)
    def git(*args,input=None):
        return subprocess.check_output(['git','-c','core.hooksPath=/dev/null','-c','commit.gpgsign=false','-C',str(checkout),*args],input=input,stderr=subprocess.DEVNULL,env={**os.environ,'GIT_AUTHOR_DATE':'2000-01-01T00:00:00Z','GIT_COMMITTER_DATE':'2000-01-01T00:00:00Z'})
    git('init','--initial-branch=main')
    lock=b'Acceptance fixture lock. Not an Ayni runtime compatibility claim.\n'
    checkout.joinpath('.ayni.lock').write_bytes(lock)
    checkout.joinpath('example.txt').write_text('Original source.\n')
    git('add','.')
    git('-c','user.name=Task Fixture','-c','user.email=fixture@example.invalid','commit','-m','test: freeze task source fixture')
    commit=git('rev-parse','HEAD').decode().strip()
    objects=git('rev-list','--objects','--no-object-names','HEAD')
    pack=git('pack-objects','--stdout',input=objects)
    root.joinpath('source.pack').write_bytes(pack)
    root.joinpath('source.json').write_text(json.dumps({'source_commit':commit,'lock_digest':'sha256:'+hashlib.sha256(lock).hexdigest(),'source_pack_digest':'sha256:'+hashlib.sha256(pack).hexdigest()},indent=2)+'\n')
