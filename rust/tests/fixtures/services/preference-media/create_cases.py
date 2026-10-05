import json
from pathlib import Path
out=[]
a='/api/user-prefs/avatar';s='/api/user-prefs/notify-sound-custom';g='/api/avatar'
png='89504e470d0a1a0a'; wav='52494646ffffffff57415645'
initial={'avatar':'https://example.invalid/existing.png','notify_sound':{'enabled':True,'type':'custom','custom_file':'<root>/notify_sound_custom.bin','custom_mime':'audio/old'}}
def add(name,path,method='PUT',hex='',**kw):
 d=dict(name=name,path=path,method=method,body={'hex':hex},initial=initial)
 d.update(kw);out.append(d)
images={'png':png,'jpeg':'ffd8ff','gif87':'474946383761','gif89':'474946383961','webp':'52494646ffffffff574542505650'}
audios={'aiff':'464f524dffffffff41494646','mpeg':'494433','midi':'4d54686400000006','wave':wav}
for name,magic in images.items():
 add('avatar-put-'+name,a,hex=magic,content_type='text/html')
 add('avatar-get-'+name,g,'GET',initial={'avatar':'<root>/user_avatar.bin'},media={'avatar':{'hex':magic}})
 for cut in range(1,len(magic)//2):add(f'avatar-truncated-{name}-{cut}',a,hex=magic[:2*cut])
for name,magic in audios.items():
 add('sound-put-'+name,s,hex=magic,content_type='image/png')
 for cut in range(1,len(magic)//2):add(f'sound-truncated-{name}-{cut}',s,hex=magic[:2*cut])
rejected={'empty':'','html':b'<html>synthetic'.hex(),'svg':b'<?xml version="1.0"?><svg/>'.hex(),'text':b'synthetic plain text'.hex(),'ogg':b'OggS\0'.hex(),'mp3-frame':'fffb9000','flac':b'fLaC'.hex(),'aac':'fff15080','webm':'1a45dfa3','bmp':b'BM'.hex(),'icon':'00000100','pdf':b'%PDF-1.7'.hex(),'mp4':'00000010667479706d70343200000000','nul-png':'00'+png,'space-wave':'20'+wav,'lower-id3':b'id3'.hex()}
for name,magic in rejected.items():
 add('avatar-reject-'+name,a,hex=magic,content_type='image/png')
 add('sound-reject-'+name,s,hex=magic,content_type='audio/ogg')
for name,magic in images.items():add('sound-reject-image-'+name,s,hex=magic)
for name,magic in audios.items():add('avatar-reject-audio-'+name,a,hex=magic)
for path,label,limit,magic in [(a,'avatar',5*1024*1024,png),(s,'sound',2*1024*1024,wav)]:
 for delta in [-1,0,1]:add(f'{label}-limit-{delta}',path,body={'hex':magic,'pad_to':limit+delta})
 add(label+'-invalid-over-limit',path,body={'hex':'','pad_to':limit+1})
 add(label+'-save-failure-published',path,hex=magic,fail_save=True,media={label:{'hex':'deadbeef'}})
 add(label+'-write-failure-unpublished',path,hex=magic,media={label:{'directory':True}})
 add(label+'-missing-root',path,hex=magic,missing_root=True)
 add(label+'-replace-shorter',path,hex=magic,media={label:{'hex':'00','pad_to':128}})
for method in ['POST','GET','HEAD','PATCH']:
 add('avatar-upload-method-'+method,a,method,hex=png)
for method in ['PUT','DELETE','HEAD']:
 add('avatar-get-method-'+method,g,method,hex=png)
for method in ['DELETE','POST','HEAD']:
 add('sound-method-'+method,s,method,hex=wav)
for pref,name in [('', 'unset'),('https://example.invalid/image.png','https'),('http://example.invalid/image.png','http'),('<root>/config.yaml','other-fixed-file'),('<root>/missing.bin','other-file'),('<root>/user_avatar.bin','missing')]:
 add('avatar-get-'+name,g,'GET',initial={'avatar':pref})
for pref,name in [('<root>/x/../user_avatar.bin','clean-parent'),('<root>/./user_avatar.bin','clean-dot'),(' <root>/user_avatar.bin ','untrimmed'),('<root>/USER_AVATAR.BIN','case')]:
 if name!='case':add('avatar-get-'+name,g,'GET',initial={'avatar':pref},media={'avatar':{'hex':png}})
for hex,name in [('', 'empty'),(b'<html>synthetic'.hex(),'corrupt'),(wav,'audio')]:
 add('avatar-get-'+name,g,'GET',initial={'avatar':'<root>/user_avatar.bin'},media={'avatar':{'hex':hex}})
add('avatar-get-directory',g,'GET',initial={'avatar':'<root>/user_avatar.bin'},media={'avatar':{'directory':True}})
add('avatar-get-oversize-existing',g,'GET',initial={'avatar':'<root>/user_avatar.bin'},media={'avatar':{'hex':png,'pad_to':5*1024*1024+1}})
for mime,name in [('', 'empty-mime'),('text/html','invalid-mime'),('audio/ogg','historical-ogg'),(' AuDiO/Custom ; codec=synthetic ','historical-spelling'),('audio/','prefix-only')]:
 add('sound-get-'+name,s,'GET',initial={'notify_sound':{'custom_file':'<root>/another.bin','custom_mime':mime}},media={'sound':{'hex':b'<html>synthetic'.hex()}})
add('sound-get-missing',s,'GET');add('sound-get-directory',s,'GET',media={'sound':{'directory':True}})
add('sound-get-empty',s,'GET',media={'sound':{'hex':''}})
add('sound-get-oversize-existing',s,'GET',media={'sound':{'hex':wav,'pad_to':2*1024*1024+1}})
for name,fail in [('success',False),('save-failure',True)]:
 add('avatar-delete-'+name,a,'DELETE',body={'hex':'ff','pad_to':5*1024*1024+1},fail_save=fail,media={'avatar':{'hex':png},'sound':{'hex':wav}})
out.append({'name':'avatar-replace-write-only','path':'/api/user-prefs/avatar','method':'PUT','body':{'hex':'474946383961'},'initial':{},'media':{'avatar':{'hex':'00','pad_to':128,'write_only':True}}})
Path('rust/tests/fixtures/services/preference-media/cases.json').write_text(json.dumps(out,indent=2)+'\n');print(len(out))
