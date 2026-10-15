import fs from 'node:fs'
import { demoProject } from '../src/lib/model.ts'
const directory = 'resources/example'
fs.mkdirSync(directory, { recursive: true })
const base = demoProject()
fs.writeFileSync(`${directory}/project.json`, JSON.stringify(base, null, 2) + '\n')
const python = structuredClone(base)
python.id = 'temperature-python'
python.name += ' · Python'
python.python = { runtimeId: '3.14.8-win_amd64', runtimeSha256: 'a93abe456ab01bd96d7a085b3cdb6566b3063f4241360d114142fbdb07f0a310', dependencyLock: [] }
const flow = python.flows.find(f => f.id === 'flow_read')
flow.nodes.splice(2, 0,
  { id: 'read_python', kind: 'python', label: 'Python conversion', position: { x: 550, y: 130 }, config: {
    code: 'def transform(msg):\n    import math\n    raw = float(msg["payload"]["rawTemperature"])\n    if not math.isfinite(raw):\n        raise ValueError("Invalid temperature")\n    msg["payload"] = {"temperature": raw / 10}\n    print("temperature:", msg["payload"]["temperature"])\n    return msg\n', timeoutMs: 5000, memoryMb: 256 } },
  { id: 'read_condition', kind: 'condition', label: 'Valid range', position: { x: 800, y: 130 }, config: { left: { source: 'message', path: 'payload.temperature' }, operator: 'gte', right: { source: 'literal', value: -273.15 } } },
)
flow.edges = flow.nodes.slice(1).map((node, i) => ({ id: `edge_read_${i}`, source: flow.nodes[i].id, sourcePort: flow.nodes[i].kind === 'condition' ? 'true' : 'out', target: node.id, targetPort: 'in' }))
flow.nodes.forEach((node,i) => node.position = { x: 50+i*250, y:130 })
fs.writeFileSync(`${directory}/python-project.json`, JSON.stringify(python, null, 2) + '\n')
