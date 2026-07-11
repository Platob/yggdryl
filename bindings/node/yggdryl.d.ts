/* Type surface for the hand-written `yggdryl.js` namespace map (see that file).
 *
 * Re-exports the NAPI-generated declarations and adds the one alias the native layer
 * cannot: `buffer.U8Buffer` is `io.ByteBuffer` (the `u8` buffer *is* the byte store).
 */

export * from './index'
import { io } from './index'

declare module './index' {
  namespace buffer {
    /** The `u8` buffer *is* the byte store: `U8Buffer` is `io.ByteBuffer` (one type). */
    const U8Buffer: typeof io.ByteBuffer
    type U8Buffer = io.ByteBuffer
  }
}
