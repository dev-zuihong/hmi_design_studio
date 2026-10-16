import { createWidget, emptyProject, type Project } from '../../src/lib/model'

export function testProject(): Project {
  const project = emptyProject()
  const button = createWidget('button')
  button.id = 'button-test'
  project.pages[0]!.widgets.push(button)
  project.connections.push({
    id: 'connection-test',
    name: 'Test connection',
    port: '',
    baudRate: 115200,
    dataBits: 8,
    stopBits: 1,
    parity: 'none',
    protocolId: '',
    protocolConfig: {},
  })
  return project
}
