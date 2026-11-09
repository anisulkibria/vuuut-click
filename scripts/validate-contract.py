#!/usr/bin/env python3
"""Validate OpenAPI and, optionally, real responses without printing account data."""
import argparse
import http.cookiejar
import json
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request
from jsonschema import Draft202012Validator, FormatChecker
from openapi_spec_validator import validate

ROOT = Path(__file__).resolve().parent.parent
METHODS = {'get', 'post', 'put', 'patch', 'delete', 'options'}

def response_object(document, response):
    if "$ref" in response:
        return document["components"]["responses"][response["$ref"].rsplit("/",1)[1]]
    return response

def check_contract(document):
    validate(document)
    count = 0
    for path, item in document['paths'].items():
        for method, op in item.items():
            if method not in METHODS:
                continue
            count += 1
            assert op.get('operationId'), (method, path, 'missing operationId')
            for code, response in op['responses'].items():
                response = response_object(document, response)
                if code == '204' or code.startswith('3'):
                    assert not response.get('content'), (method, path, code)
                elif code.startswith('2') and 'empty body' not in response['description']:
                    assert response.get('content'), (method, path, code, 'missing content')
                elif code.startswith(('4', '5')) or code == 'default':
                    schema = response['content']['application/json']['schema']
                    assert schema == {'$ref': '#/components/schemas/ApiError'}, (method, path, code)
    # Catch undocumented JSON routes and nonexistent method/path pairs.
    routes = set()
    for filename in ['lib.rs', 'native.rs', 'explore.rs', 'costs.rs', 'commercial.rs']:
        source = (ROOT/'crates/click-server/src'/filename).read_text()
        for match in re.finditer(r'\.route\(\s*"([^"]+)"\s*,', source):
            path = match[1]
            if not (path.startswith(('/v1/', '/native/v1/', '/owner/v1/')) or path in ['/healthz', '/readyz']):
                continue
            # Balance the route call so helpers following a router are excluded.
            depth = 1
            quoted = False
            escaped = False
            end = match.end()
            while depth:
                char = source[end]
                if quoted:
                    if escaped: escaped = False
                    elif char == chr(92): escaped = True
                    elif char == chr(34): quoted = False
                elif char == chr(34): quoted = True
                elif source[end:end+2] == '//':
                    end = source.index(chr(10), end)
                elif char == '(': depth += 1
                elif char == ')': depth -= 1
                end += 1
            body = source[match.end():end-1]
            for method in re.findall(r'\b(get|post|put|patch|delete|options)\(', body):
                routes.add((method, re.sub(r'\{[^}]+\}', '{}', path)))
    documented = {(m, re.sub(r'\{[^}]+\}', '{}', p)) for p, item in document['paths'].items() for m in item if m in METHODS and (p.startswith(('/v1/', '/native/v1/', '/owner/v1/')) or p in ['/healthz', '/readyz'])}
    assert routes == documented, {'undocumented': sorted(routes-documented), 'nonexistent': sorted(documented-routes)}
    return count

def validate_response(document, template, method, status, content_type, body):
    operation = document['paths'][template][method]
    response = response_object(document, operation['responses'].get(str(status), operation['responses']['default']))
    if not response.get('content'):
        assert not body, (method, template, status, 'unexpected body')
        return
    media = content_type.split(';')[0]
    assert media in response['content'], (method, template, status, 'unexpected media type')
    schema = dict(response['content'][media]['schema'])
    schema['components'] = document['components']
    value = json.loads(body) if media == 'application/json' else body.decode()
    errors = list(Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(value))
    # Report only paths/keywords, never values containing account information.
    assert not errors, (method, template, status, [(list(e.absolute_path), e.validator) for e in errors[:5]])

def smoke(document, base, accounts_file):
    url = urllib.parse.urlsplit(base)
    assert url.scheme == 'http' and url.hostname in ['localhost', '127.0.0.1', '::1'], 'Local validation only'
    assert not url.username and not url.password and not url.query and not url.fragment
    base = base.rstrip('/')
    client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    checked = 0
    def request(template, actual=None, method='get', data=None, expected=200):
        nonlocal checked
        req = urllib.request.Request(base+(actual or template), method=method.upper(), data=None if data is None else json.dumps(data).encode(), headers={'Origin': base, 'Content-Type':'application/json','Accept':'application/json'})
        try:
            result = client.open(req, timeout=35)
        except urllib.error.HTTPError as error:
            result = error
        with result:
            status = result.code
            assert status == expected, (method, template, status, expected)
            body = result.read()
            validate_response(document, template, method, status, result.headers.get('Content-Type',''), body)
            checked += 1
            return json.loads(body) if result.headers.get('Content-Type','').startswith('application/json') else None
    request('/healthz'); request('/readyz')
    served = request('/v1/openapi.json')
    assert served == document, 'Running binary embeds an older API contract; rebuild/restart first'
    request('/v1/me', expected=401)
    if accounts_file:
        accounts = json.loads(Path(accounts_file).read_text())
        request('/v1/auth/login', method='post', data=accounts['user'])
        request('/v1/me'); orgs=request('/v1/organizations'); sites=request('/v1/websites')
        site = accounts['website_id']; org = next(w['organization_id'] for w in sites if w['id']==site)
        request('/v1/organizations/{id}/usage', f'/v1/organizations/{org}/usage')
        for suffix in ['overview','breakdown/countries','goals','funnels','journeys','vitals','revenue','comparison','content','acquisition','retention','vitals/comparison','newsletters','events','campaign-costs','native-report','exports']:
            template_suffix = 'breakdown/{dimension}' if suffix.startswith('breakdown/') else suffix
            # Retention is the advanced report GET; settings write is PUT, both are documented.
            expected = 404 if suffix == 'exports' and next(o['role'] for o in orgs if o['id']==org) == 'viewer' else 200
            request('/v1/websites/{id}/'+template_suffix, f'/v1/websites/{site}/{suffix}', expected=expected)
        request('/owner/v1/me', expected=401)
        request('/v1/auth/logout', method='post', expected=204)
    return checked

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base');parser.add_argument('--accounts',type=Path)
    args=parser.parse_args();document=json.loads((ROOT/'docs/openapi.json').read_text())
    count=check_contract(document)
    live=smoke(document,args.base,args.accounts) if args.base else 0
    print(json.dumps({'openapi_operations':count,'route_coverage':True,'live_responses_validated':live}))
if __name__=='__main__':main()
