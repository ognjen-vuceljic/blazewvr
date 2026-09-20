%dw 2.0
import shout from StringUtils
output application/json
---
{
    shouted: shout("hello from a module")
}
