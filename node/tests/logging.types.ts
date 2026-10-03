import { logging } from '..'
import type { FileHandler, Logger, LoggingBasicConfig, LoggingHandlerInput } from '..'

const logger: Logger = logging.getLogger('typed')
logger.setLevel(logging.INFO)
logger.setLevel('debug')
const effective: number = logger.getEffectiveLevel()
const enabled: boolean = logger.isEnabledFor('WARNING')
logger.propagate = false
logger.deduplicating = true
logger.deduplicating = null
const deduplicating: boolean = logger.isDeduplicating()
void deduplicating
const file: FileHandler = new logging.FileHandler('typed.log', { mode: 'append', capacity: 4096, flushLevel: 'ERROR' })
file.setFormatter(new logging.Formatter('%(message)s', { datefmt: '%F' }))
const handlers: LoggingHandlerInput[] = [file, new logging.StreamHandler('stderr'), new logging.NullHandler()]
const config: LoggingBasicConfig = { level: 'INFO', handlers, force: true }
logging.basicConfig(config)
logging.disable()
logging.shutdown()
void effective
void enabled

// @ts-expect-error a message is text
logger.info(42)