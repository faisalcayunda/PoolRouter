# Inisialisasi IAM dev untuk object storage arsip (SHAA-361 butir 6, 7, 9; ADR-0006 rev 4 §8 butir 5, §8.1 butir 9 dan 13).
# Membuat tiga identitas bucket, masing-masing dengan kebijakan sendiri:
#   - arsip   (job arsip):     PutObject, GetObject pada bucket arsip. Tanpa Delete*, tanpa versioning/lifecycle.
#   - restore (proses restore): GetObject dan GetObjectVersion (baca per versi). Tanpa tulis, tanpa Delete*.
#   - rotasi  (job rotasi KEK): tanpa kebijakan apa pun (tanpa akses bucket). Ditolak untuk semua aksi.
# Semua aksi yang tidak tercantum ditolak secara implisit oleh RustFS (IAM default-deny), termasuk
# DeleteObject, DeleteObjectVersion, BypassGovernanceRetention, PutBucketVersioning, dan lifecycle.
# Stdlib saja; panggilan admin RustFS ditandatangani SigV4.
# Kredensial hanya dari /run/secrets/*. Tidak ada nilai rahasia di file ini.
import datetime, hashlib, hmac, json, os, sys, time, urllib.error, urllib.parse, urllib.request

HOST = os.environ.get("OBJSTORE_HOST", "object-storage:9000")
REGION = "us-east-1"
BUCKET = os.environ.get("POOLROUTER_ARSIP_BUCKET", "poolrouter-arsip-dev")


def secret(name):
    with open(f"/run/secrets/{name}", encoding="utf-8") as f:
        return f.read().strip()


ROOT_AK = secret("objstore_root_user")
ROOT_SK = secret("objstore_root_password")


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


def _policy(actions):
    return {"Version": "2012-10-17", "Statement": [{
        "Effect": "Allow",
        "Action": actions,
        "Resource": [f"arn:aws:s3:::{BUCKET}", f"arn:aws:s3:::{BUCKET}/*"]}]}


# Identitas: (nama policy atau None, access key env, default access key, nama secret).
# Policy None = user tanpa lampiran policy (default-deny).
IDENTITAS = [
    ("arsip-put-get", "OBJSTORE_ARSIP_ACCESS_KEY", "poolrouter-arsip-job", "objstore_arsip_secret",
     _policy(["s3:PutObject", "s3:GetObject", "s3:ListBucket", "s3:GetBucketLocation"])),
    ("restore-get-version", "OBJSTORE_RESTORE_ACCESS_KEY", "poolrouter-arsip-restore", "objstore_restore_secret",
     _policy(["s3:GetObject", "s3:GetObjectVersion", "s3:ListBucket", "s3:GetBucketLocation"])),
    (None, "OBJSTORE_ROTASI_ACCESS_KEY", "poolrouter-arsip-rotasi", "objstore_rotasi_secret", None),
]


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

    for policy_name, ak_env, ak_default, secret_name, policy in IDENTITAS:
        access_key = os.environ.get(ak_env, ak_default)
        sk = secret(secret_name)
        if policy_name is not None:
            s, body = admin("PUT", "/rustfs/admin/v3/add-canned-policy", {"name": policy_name},
                            json.dumps(policy).encode())
            if not ok(s):
                print(f"add-canned-policy {policy_name} gagal: {s} {body}", file=sys.stderr); return 1
            print("policy", policy_name, "ok")

        s, body = admin("PUT", "/rustfs/admin/v3/add-user", {"accessKey": access_key},
                        json.dumps({"secretKey": sk, "status": "enabled"}).encode())
        if not ok(s) and s != 409:
            print(f"add-user {access_key} gagal: {s} {body}", file=sys.stderr); return 1
        print("user", access_key, "ok (status", s, ")")

        if policy_name is not None:
            s, body = admin("PUT", "/rustfs/admin/v3/set-user-or-group-policy",
                            {"policyName": policy_name, "userOrGroup": access_key, "isGroup": "false"})
            if not ok(s):
                print(f"attach {policy_name} gagal: {s} {body}", file=sys.stderr); return 1
            print("attach", policy_name, "->", access_key, "ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
