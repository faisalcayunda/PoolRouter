# Inisialisasi IAM dev untuk object storage arsip (SHAA-334, ADR-0006 D6).
# Membuat policy worker tanpa Delete dan user worker, lalu melampirkan policy.
# Stdlib saja; panggilan admin RustFS ditandatangani SigV4.
# Kredensial hanya dari /run/secrets/*. Tidak ada nilai rahasia di file ini.
import datetime, hashlib, hmac, json, os, sys, time, urllib.error, urllib.parse, urllib.request

HOST = os.environ.get("OBJSTORE_HOST", "object-storage:9000")
REGION = "us-east-1"
BUCKET = os.environ.get("POOLROUTER_ARSIP_BUCKET", "poolrouter-arsip-dev")
WORKER_AK = os.environ.get("OBJSTORE_WORKER_ACCESS_KEY", "poolrouter-worker-arsip")
POLICY_NAME = "worker-arsip-nodelete"


def secret(name):
    with open(f"/run/secrets/{name}", encoding="utf-8") as f:
        return f.read().strip()


ROOT_AK = secret("objstore_root_user")
ROOT_SK = secret("objstore_root_password")
WORKER_SK = secret("objstore_worker_secret")


def _h(key, msg):
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def admin(method, path, query, body=b""):
    now = datetime.datetime.now(datetime.timezone.utc)
    amz = now.strftime("%Y%m%dT%H%M%SZ")
    date = amz[:8]
    payload = hashlib.sha256(body).hexdigest()
    cq = "&".join(f"{urllib.parse.quote(k, safe='')}={urllib.parse.quote(v, safe='')}" for k, v in sorted(query.items()))
    hdrs = {"host": HOST, "x-amz-date": amz, "x-amz-content-sha256": payload}
    signed = ";".join(sorted(hdrs))
    canon_h = "".join(f"{k}:{hdrs[k]}\n" for k in sorted(hdrs))
    creq = "\n".join([method, path, cq, canon_h, signed, payload])
    scope = f"{date}/{REGION}/s3/aws4_request"
    sts = "\n".join(["AWS4-HMAC-SHA256", amz, scope, hashlib.sha256(creq.encode()).hexdigest()])
    k = _h(_h(_h(_h(("AWS4" + ROOT_SK).encode(), date), REGION), "s3"), "aws4_request")
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()
    auth = f"AWS4-HMAC-SHA256 Credential={ROOT_AK}/{scope}, SignedHeaders={signed}, Signature={sig}"
    req = urllib.request.Request(
        f"http://{HOST}{path}?{cq}", data=body or None, method=method,
        headers={"x-amz-date": amz, "x-amz-content-sha256": payload, "Authorization": auth,
                 "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, r.read().decode()[:300]
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:300]
    # Kesalahan jaringan dibiarkan naik; compose akan menandai service gagal.


POLICY = {"Version": "2012-10-17", "Statement": [{
    "Effect": "Allow",
    "Action": ["s3:PutObject", "s3:GetObject", "s3:ListBucket", "s3:GetBucketLocation",
               "s3:GetBucketVersioning", "s3:GetObjectVersion", "s3:ListBucketVersions"],
    "Resource": [f"arn:aws:s3:::{BUCKET}", f"arn:aws:s3:::{BUCKET}/*"]}]}


def ok(status):
    return status in (200, 204)


def main():
    # Tunggu admin API siap (object-storage sudah healthy, tapi tetap dicek sekali lagi).
    for _ in range(20):
        try:
            status, _ = admin("GET", "/rustfs/admin/v3/info", {})
            if status < 500:
                break
        except OSError:
            pass
        time.sleep(1)

    s, body = admin("PUT", "/rustfs/admin/v3/add-canned-policy", {"name": POLICY_NAME},
                    json.dumps(POLICY).encode())
    if not ok(s):
        print(f"add-canned-policy gagal: {s} {body}", file=sys.stderr); return 1
    print("policy", POLICY_NAME, "ok")

    s, body = admin("PUT", "/rustfs/admin/v3/add-user", {"accessKey": WORKER_AK},
                    json.dumps({"secretKey": WORKER_SK, "status": "enabled"}).encode())
    if not ok(s) and s != 409:
        print(f"add-user gagal: {s} {body}", file=sys.stderr); return 1
    print("user", WORKER_AK, "ok (status", s, ")")

    s, body = admin("PUT", "/rustfs/admin/v3/set-user-or-group-policy",
                    {"policyName": POLICY_NAME, "userOrGroup": WORKER_AK, "isGroup": "false"})
    if not ok(s):
        print(f"attach gagal: {s} {body}", file=sys.stderr); return 1
    print("attach", POLICY_NAME, "->", WORKER_AK, "ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
