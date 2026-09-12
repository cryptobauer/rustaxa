#!/usr/bin/env python3
"""Bounded read-only availability probe; records observations, never imports data.

One request per documented endpoint, 15-second timeout and 256 KiB response cap.
Failures are evidence about this environment, not proof of global unavailability.
"""
import concurrent.futures, datetime, hashlib, json, pathlib, urllib.request
HERE=pathlib.Path(__file__).resolve().parent

def probe(network,kind):
    url=f'https://rpc.{network}.taraxa.io' if kind=='rpc' else f'https://snapshots.taraxa.io/api?network={network}'
    payload=[{'jsonrpc':'2.0','id':i,'method':m,'params':p} for i,(m,p) in enumerate([('eth_chainId',[]),('taraxa_getVersion',[]),('eth_getBlockByNumber',['0x0',False]),('eth_getBlockByNumber',['latest',False]),('eth_getBalance',['0x00000000000000000000000000000000000000fe','0x0'])])]
    request=urllib.request.Request(url,data=json.dumps(payload).encode() if kind=='rpc' else None,headers={'Content-Type':'application/json','User-Agent':'Rustaxa-feasibility/1'})
    row={'network':network,'kind':kind,'url':url}
    try:
        with urllib.request.urlopen(request,timeout=15) as response:
            body=response.read(262145)
            if len(body)>262144:raise ValueError('response limit exceeded')
            row.update(status=response.status,sha256=hashlib.sha256(body).hexdigest(),body=body.decode())
    except Exception as error:row['error']=str(error)
    return row

def main():
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        results=list(pool.map(lambda x:probe(*x),[(n,k) for n in ['mainnet','testnet'] for k in ['rpc','snapshot']]))
    output={'observed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scope':'availability only; no archive acquired or peer messages sent','results':results}
    (HERE/'fixtures/network_availability.json').write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps(output,indent=2))
if __name__=='__main__':main()
