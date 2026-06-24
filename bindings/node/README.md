# yggdryl (Node.js)

Node.js bindings for [**yggdryl**](https://github.com/Platob/yggdryl), backed by
the Rust `yggdryl` core crate (via [napi-rs](https://napi.rs)).

## Build

```bash
npm install
npm run build            # napi build --platform --release
npm test                 # node --test
```

## Usage

```javascript
const { Uri, Url } = require('yggdryl')

const uri = new Uri('urn:isbn:0451450523')
console.log(uri.scheme)        // urn
console.log(uri.path)          // isbn:0451450523

const url = new Url('https://user:pw@example.com:8443/api?v=1#top')
console.log(url.host)          // example.com
console.log(url.port)          // 8443
console.log(url.username)      // user
console.log(url.toString())    // https://user:pw@example.com:8443/api?v=1#top
```

Invalid input throws.
