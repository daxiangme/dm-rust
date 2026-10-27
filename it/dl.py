import subprocess, os, sys, threading, time
URL="https://download.dameng.com/eco/dm8/dm8_20241022_x86_rh6_64_single.tar"
SIZE=806840320; CH=8*1024*1024
n=(SIZE+CH-1)//CH
os.makedirs("parts",exist_ok=True)
q=list(range(n)); lock=threading.Lock()
def worker():
    while True:
        with lock:
            if not q: return
            i=q.pop(0)
        s=i*CH; e=min(SIZE-1,s+CH-1); f=f"parts/c{i:04d}"
        if os.path.exists(f) and os.path.getsize(f)==e-s+1: continue
        while True:
            r=subprocess.run(["curl","-s","-m","120","-r",f"{s}-{e}","-o",f+".tmp","-w","%{http_code}",URL],capture_output=True,text=True)
            if r.stdout=="206" and os.path.getsize(f+".tmp")==e-s+1:
                os.rename(f+".tmp",f); break
            time.sleep(3)
ts=[threading.Thread(target=worker) for _ in range(2)]
[t.start() for t in ts]; [t.join() for t in ts]
print("done")
