# Regenerates docs/games.md from the game catalogue.
# Usage: python tools/gen_games_md.py crates/lighthouse-core/src/catalog.rs docs/games.md
import re, sys
src = open(sys.argv[1], encoding='utf-8').read()
rows = []
pat = re.compile(r'id: "([^"]+)", aliases: &\[([^\]]*)\], name: "((?:[^"\\]|\\.)*)", short: "[^"]*", '
                 r'protocol: Protocol::(\w+), game_port: (\d+), query_port: QueryPort::(\w+(?:\(-?\d+\))?), support: Support::(\w+)')
for m in pat.finditer(src):
    gid, aliases, name, proto, gp, q, sup = m.groups()
    gp = int(gp)
    if q == 'Same':
        qp = str(gp)
    elif q.startswith('Fixed'):
        qp = q[6:-1]
    else:
        off = int(q[7:-1])
        qp = f'{gp + off} (port {"+" if off >= 0 else "-"} {abs(off)})'
    al = ', '.join(f'`{a.strip().strip(chr(34))}`' for a in aliases.split(',') if a.strip())
    proto_name = {'Steam': 'Steam query', 'MinecraftJava': 'Minecraft Java ping', 'MinecraftBedrock': 'Minecraft Bedrock ping',
                  'FiveM': 'FiveM HTTP'}[proto]
    rows.append((sup, name.replace('\\"', '"'), gid, al, proto_name, gp, qp))
rows.sort(key=lambda r: (r[0] != 'Full', r[1].lower()))
full = [r for r in rows if r[0] == 'Full']
exp = [r for r in rows if r[0] != 'Full']
out = ['# Supported games', '',
       'Use the **id** (or an alias) as `game` in `lighthouse.toml`, or pick the game by name in `lighthouse setup`.',
       'Run `lighthouse games` to print this list, or `lighthouse query <id> <host> [port]` to test a server.', '',
       'The **port** is the default port players connect to. The **query** port is where Lighthouse asks for the',
       'status; set `query_port` on a server if yours differs.', '',
       '## Fully supported', '',
       'Tested against live servers, with game-specific details (in-game time, blood moon, correct version).', '',
       '| Game | id | Aliases | Protocol | Port | Query |', '|---|---|---|---|---|---|']
for r in full:
    out.append(f'| {r[1]} | `{r[2]}` | {r[3]} | {r[4]} | {r[5]} | {r[6]} |')
out += ['', '## Experimental', '',
        'These use the generic adapter for their protocol. They should work, but their default ports come from',
        'public query documentation and have not been checked on a live server yet. Reports and fixes are welcome.', '',
        '| Game | id | Aliases | Protocol | Port | Query |', '|---|---|---|---|---|---|']
for r in exp:
    out.append(f'| {r[1]} | `{r[2]}` | {r[3]} | {r[4]} | {r[5]} | {r[6]} |')
out += ['', f'{len(rows)} games in total.', '']
open(sys.argv[2], 'w', encoding='utf-8', newline='\n').write('\n'.join(out))
print(len(full), 'full,', len(exp), 'experimental')
