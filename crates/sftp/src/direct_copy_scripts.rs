pub(crate) const SENDER_SCRIPT: &str = r#"
import base64,json,os,socket,stat,struct,sys
host=base64.b64decode(sys.argv[1]).decode()
port,token_hex,paths_b64=int(sys.argv[2]),sys.argv[3],sys.argv[4]
paths=json.loads(base64.b64decode(paths_b64))
def scan(path,rel):
    info=os.lstat(path)
    if stat.S_ISLNK(info.st_mode): raise RuntimeError("symbolic links are not supported")
    if stat.S_ISDIR(info.st_mode):
        total=0
        for name in sorted(os.listdir(path)):
            total+=scan(os.path.join(path,name),os.path.join(rel,name))
        return total
    if not stat.S_ISREG(info.st_mode): raise RuntimeError("special files are not supported")
    return info.st_size
total=sum(scan(path,os.path.basename(path.rstrip("/"))) for path in paths)
print("NAVOP_TOTAL "+str(total),flush=True)
s=socket.create_connection((host,port),3)
s.sendall(bytes.fromhex(token_hex))
def frame(value):
    data=json.dumps(value,separators=(",",":")).encode()
    s.sendall(struct.pack("!I",len(data))+data)
def send(path,rel):
    info=os.lstat(path)
    if stat.S_ISDIR(info.st_mode):
        frame({"kind":"dir","path":rel,"mode":info.st_mode & 0o7777,"mtime":info.st_mtime})
        for name in sorted(os.listdir(path)):
            send(os.path.join(path,name),os.path.join(rel,name))
    else:
        frame({"kind":"file","path":rel,"size":info.st_size,"mode":info.st_mode & 0o7777,"mtime":info.st_mtime})
        with open(path,"rb") as source:
            while True:
                data=source.read(262144)
                if not data: break
                s.sendall(data)
                print("NAVOP_PROGRESS "+str(len(data)),flush=True)
for path in paths: send(path,os.path.basename(path.rstrip("/")))
s.sendall(struct.pack("!I",0))
s.close()
"#;

pub(crate) const RECEIVER_SCRIPT: &str = r#"
import base64,hmac,json,os,socket,stat,struct,sys
root=base64.b64decode(sys.argv[1]).decode()
token_hex=sys.argv[2]
os.makedirs(root,exist_ok=True)
root_real=os.path.realpath(root)
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
s.bind(("0.0.0.0",0)); s.listen(1)
print("NAVOP_READY "+str(s.getsockname()[1]),flush=True)
s.settimeout(120)
c,_=s.accept()
token=bytes.fromhex(token_hex)
received=b""
while len(received)<len(token):
    part=c.recv(len(token)-len(received))
    if not part: raise RuntimeError("connection closed before authentication")
    received+=part
if not hmac.compare_digest(received,token): raise RuntimeError("invalid transfer token")
def read_exact(size):
    data=b""
    while len(data)<size:
        part=c.recv(size-len(data))
        if not part: raise RuntimeError("connection closed during transfer")
        data+=part
    return data
def safe_path(relative):
    if not relative or os.path.isabs(relative) or chr(0) in relative: raise RuntimeError("invalid relative path")
    candidate=os.path.abspath(os.path.join(root,relative))
    parent_real=os.path.realpath(os.path.dirname(candidate))
    if os.path.commonpath((root_real,parent_real)) != root_real: raise RuntimeError("path escapes destination")
    if os.path.lexists(candidate) and os.path.islink(candidate): raise RuntimeError("symbolic link destination is not supported")
    return candidate
while True:
    size=struct.unpack("!I",read_exact(4))[0]
    if size==0: break
    item=json.loads(read_exact(size).decode())
    path=safe_path(item["path"])
    if item["kind"]=="dir":
        os.makedirs(path,exist_ok=True); os.chmod(path,item["mode"])
    elif item["kind"]=="file":
        os.makedirs(os.path.dirname(path),exist_ok=True)
        remaining=item["size"]
        with open(path,"wb") as target:
            while remaining:
                data=c.recv(min(262144,remaining))
                if not data: raise RuntimeError("connection closed while writing file")
                target.write(data); remaining-=len(data)
        os.chmod(path,item["mode"]); os.utime(path,(item["mtime"],item["mtime"]))
    else: raise RuntimeError("unknown transfer entry")
c.close(); s.close()
"#;
