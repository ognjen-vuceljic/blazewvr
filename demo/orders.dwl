%dw 2.0
output application/json
---
{
    orderCount: sizeOf(payload.orders),
    total: sum(payload.orders.price),
    highValue: payload.orders filter ((o) -> o.price > 50)
}
