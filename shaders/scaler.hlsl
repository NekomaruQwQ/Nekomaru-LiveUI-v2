// Simple texture scaler using bilinear sampling
// Resizes input texture to output render target size

Texture2D<float4> inputTexture : register(t0);
SamplerState linearSampler : register(s0);

struct VSOutput {
    float4 position : SV_POSITION;
    float2 texcoord : TEXCOORD0;
};

// Vertex shader - generates fullscreen triangle
VSOutput VSMain(uint vertexID : SV_VertexID) {
    VSOutput output;

    // Generate fullscreen triangle coordinates
    // vertexID: 0 -> (-1, -1), 1 -> (3, -1), 2 -> (-1, 3)
    output.texcoord = float2((vertexID << 1) & 2, vertexID & 2);
    output.position = float4(output.texcoord * float2(2, -2) + float2(-1, 1), 0, 1);

    return output;
}

// Pixel shader - sample and output
float4 PSMain(VSOutput input) : SV_TARGET {
    return inputTexture.Sample(linearSampler, input.texcoord);
}
