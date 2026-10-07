// 最小：位置完全由 SV_VertexID 造出来，**不读任何顶点属性**。
float4 VSMain(uint vid : SV_VertexID) : SV_Position {
    float2 p = (vid == 0) ? float2(-1.0, 3.0)
           : (vid == 1) ? float2(-1.0, -1.0)
                        : float2(3.0, -1.0);
    return float4(p, 0.0, 1.0);
}
float4 PSMain() : SV_Target { return float4(1.0, 0.0, 0.0, 1.0); }