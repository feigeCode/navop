import base64, hmac, hashlib, struct, sys, time

def code(secret, t=None, digits=6, period=30, algo='sha1'):
    t = int(time.time()) if t is None else t
    key = base64.b32decode(secret.upper() + '=' * (-len(secret) % 8))
    counter = struct.pack('>Q', t // period)
    digest = hmac.new(key, counter, getattr(hashlib, algo)).digest()
    offset = digest[-1] & 0x0F
    value = struct.unpack('>I', digest[offset:offset + 4])[0] & 0x7FFFFFFF
    return str(value % (10 ** digits)).zfill(digits)

if __name__ == '__main__':
    print(code(sys.argv[1] if len(sys.argv) > 1 else 'JBSWY3DPEHPK3PXP'))
