// Contador de quadros por segundo e tempo de quadro no canto da tela (o equivalente ao OSD de FPS do ReShade).
// Escrito para este projeto; usa só a fonte `frametime` do runtime. O valor é suavizado entre os quadros.

uniform int Corner <
	ui_type = "combo";
	ui_label = "Canto";
	ui_items = "Superior esquerdo\0Superior direito\0Inferior esquerdo\0Inferior direito\0";
> = 0;

uniform int Scale <
	ui_type = "slider";
	ui_label = "Tamanho";
	ui_min = 1; ui_max = 8;
> = 3;

uniform float Smoothing <
	ui_type = "slider";
	ui_label = "Suavização";
	ui_tooltip = "Quanto cada quadro novo pesa na média (menor = mais estável).";
	ui_min = 0.005; ui_max = 1.0;
> = 0.05;

uniform float FRAME_TIME < source = "frametime"; >;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

// A média do tempo de quadro, em duas texturas de um pixel que se revezam (um passe não lê o que escreve).
texture AvgA { Width = 1; Height = 1; Format = R32F; };
texture AvgB { Width = 1; Height = 1; Format = R32F; };
sampler sAvgA { Texture = AvgA; MagFilter = POINT; MinFilter = POINT; };
sampler sAvgB { Texture = AvgB; MagFilter = POINT; MinFilter = POINT; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float PS_Update(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float previous = tex2Dfetch(sAvgA, int2(0, 0)).x;
	float now = max(FRAME_TIME, 0.01);
	return previous <= 0.0 ? now : lerp(previous, now, Smoothing);
}

float PS_Copy(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2Dfetch(sAvgB, int2(0, 0)).x;
}

// Fonte de 3x5 pixels, uma linha por três bits (de cima para baixo), do bit 14 ao 0.
// 0-9 dígitos, 10 '.', 11 F, 12 P, 13 S, 14 M, 15 espaço.
uint Glyph(int g)
{
	uint font[16] = { 0x7B6F, 0x2C97, 0x73E7, 0x73CF, 0x5BC9, 0x79CF, 0x79EF, 0x7249, 0x7BEF, 0x7BCF, 0x0002, 0x79E4, 0x7BE4, 0x79CF, 0x5FED, 0x0000 };
	return font[clamp(g, 0, 15)];
}

// O caractere `i` da linha: um número com uma casa decimal (valor*10) e a unidade.
int CharAt(int line, int i, int value)
{
	// "123.4 FPS" / "123.4 MS": quatro dígitos com ponto antes do último, espaço, unidade.
	int d3 = (value / 1000) % 10;
	int d2 = (value / 100) % 10;
	int d1 = (value / 10) % 10;
	int d0 = value % 10;
	if (i == 0) return d3 == 0 ? 15 : d3;
	if (i == 1) return (d3 == 0 && d2 == 0) ? 15 : d2;
	if (i == 2) return d1;
	if (i == 3) return 10;
	if (i == 4) return d0;
	if (i == 5) return 15;
	if (line == 0) return i == 6 ? 11 : (i == 7 ? 12 : 13);
	return i == 6 ? 14 : 13;
}

float4 PS_Draw(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	float frame_ms = tex2Dfetch(sAvgB, int2(0, 0)).x;
	int fps10 = (int)clamp(round(10000.0 / max(frame_ms, 0.01)), 0.0, 9999.0);
	int ms10 = (int)clamp(round(frame_ms * 10.0), 0.0, 9999.0);

	int cell_w = 4 * Scale;
	int cell_h = 6 * Scale;
	int columns = 9;
	int margin = 2 * Scale;
	int box_w = columns * cell_w + 2 * margin;
	int box_h = 2 * cell_h + 3 * margin / 2 + margin / 2;
	int2 size = int2(BUFFER_WIDTH, BUFFER_HEIGHT);
	int2 origin = int2((Corner == 1 || Corner == 3) ? size.x - box_w - margin : margin, (Corner >= 2) ? size.y - box_h - margin : margin);
	int2 p = int2(position.xy) - origin;
	if (p.x < 0 || p.y < 0 || p.x >= box_w || p.y >= box_h)
		return color;

	float3 result = color.rgb * 0.35;
	int2 t = p - int2(margin, margin);
	if (t.x >= 0 && t.y >= 0 && t.x < columns * cell_w && t.y < 2 * cell_h + margin / 2)
	{
		int line = t.y / (cell_h + margin / 4);
		int local_y = t.y - line * (cell_h + margin / 4);
		int col = t.x / cell_w;
		int2 pixel = int2((t.x % cell_w) / Scale, local_y / Scale);
		if (line < 2 && pixel.x < 3 && pixel.y < 5 && local_y < cell_h)
		{
			int g = CharAt(line, col, line == 0 ? fps10 : ms10);
			uint bits = Glyph(g);
			uint bit = 14u - (uint)(pixel.y * 3 + pixel.x);
			if (((bits >> bit) & 1u) != 0u)
				result = float3(1.0, 1.0, 1.0);
		}
	}
	return float4(result, color.a);
}

technique FPSCounter < ui_label = "Contador de FPS"; >
{
	pass Update { VertexShader = VS; PixelShader = PS_Update; RenderTarget = AvgB; }
	pass Copy { VertexShader = VS; PixelShader = PS_Copy; RenderTarget = AvgA; }
	pass Draw { VertexShader = VS; PixelShader = PS_Draw; }
}
