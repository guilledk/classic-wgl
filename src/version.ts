import { version } from '../package.json'

export const APP_NAME = 'classic-wgl'
export const APP_VERSION = version

export const APP_VERSION_DISPLAY = `v${APP_VERSION.replace(
  /^(\d+)\.(\d+)\.\d+-(alpha|beta)\.(\d+)$/,
  (_, major, minor, tag, num) => `${major}.${minor}${tag[0]}${num}`
)}`
