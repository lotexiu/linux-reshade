// Desempenho no canto da tela (o equivalente ao OSD de FPS do ReShade): FPS agora, média, mínimo, máximo e 1% low dos últimos 5 s, e o tempo de
// quadro. Escrito para este projeto; usa as fontes `frametime` e `fps_*` do runtime. O FPS "agora" é suavizado entre os quadros.

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

uniform bool Details <
	ui_label = "Média, mínimo, máximo e 1% low";
	ui_tooltip = "Desligado mostra só o FPS e o tempo de quadro.";
> = true;

uniform float FRAME_TIME < source = "frametime"; >;
uniform float FPS_AVG < source = "fps_avg"; >;
uniform float FPS_MIN < source = "fps_min"; >;
uniform float FPS_MAX < source = "fps_max"; >;
uniform float FPS_LOW1 < source = "fps_low1"; >;

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
// 0-9 dígitos, 10 '.', 11 F, 12 P, 13 S, 14 M, 15 espaço, 16 A, 17 V, 18 G, 19 I, 20 N, 21 X, 22 L, 23 %.
uint Glyph(int g)
{
	uint font[24] = { 0x7B6F, 0x2C97, 0x73E7, 0x73CF, 0x5BC9, 0x79CF, 0x79EF, 0x7249, 0x7BEF, 0x7BCF, 0x0002, 0x79E4, 0x7BE4, 0x79CF, 0x5FED, 0x0000,
	                  0x2BED, 0x5B6A, 0x796F, 0x7497, 0x7B6D, 0x5AAD, 0x4927, 0x52A5 };
	return font[clamp(g, 0, 23)];
}

// Os três caracteres do nome de cada linha: 0 FPS, 1 AVG, 2 MIN, 3 MAX, 4 1%L, 5 MS.
int Label(int line, int i)
{
	int labels[18] = { 11, 12, 13,  16, 17, 18,  14, 19, 20,  14, 16, 21,  1, 23, 22,  14, 13, 15 };
	return labels[line * 3 + i];
}

int Pow10(int n)
{
	return n == 0 ? 1 : (n == 1 ? 10 : (n == 2 ? 100 : 1000));
}

// O caractere `i` (0 a 8) da linha: o nome, um espaço e o valor. Os inteiros ficam à esquerda sem zeros sobrando; a linha do tempo de quadro
// (5) mostra `valor` como décimos de milissegundo ("12.3").
int CharAt(int line, int i, int value)
{
	if (i < 3) return Label(line, i);
	if (i == 3) return 15;
	int k = i - 4;
	value = clamp(value, 0, 99999);
	if (line == 5)
	{
		int digits = value >= 1000 ? 4 : (value >= 100 ? 3 : 2);
		int lead = 4 - digits; // espaços à esquerda dentro de "dd.d" com 4 dígitos
		if (k == 3) return 10;
		int pos = k < 3 ? k : k - 1; // posição do dígito sem o ponto: 0..3
		if (pos < lead) return 15;
		return (value / Pow10(3 - pos)) % 10;
	}
	int n = value >= 10000 ? 5 : (value >= 1000 ? 4 : (value >= 100 ? 3 : (value >= 10 ? 2 : 1)));
	if (k >= n) return 15;
	return (value / Pow10(n - 1 - k)) % 10;
}

float4 PS_Draw(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	float frame_ms = tex2Dfetch(sAvgB, int2(0, 0)).x;
	int values[6];
	values[0] = (int)clamp(round(10000.0 / max(frame_ms, 0.01)) / 10.0, 0.0, 9999.0);
	values[1] = (int)clamp(round(FPS_AVG), 0.0, 9999.0);
	values[2] = (int)clamp(round(FPS_MIN), 0.0, 9999.0);
	values[3] = (int)clamp(round(FPS_MAX), 0.0, 9999.0);
	values[4] = (int)clamp(round(FPS_LOW1), 0.0, 9999.0);
	values[5] = (int)clamp(round(frame_ms * 10.0), 0.0, 9999.0);

	int rows = Details ? 6 : 2;
	int cell_w = 4 * Scale;
	int cell_h = 6 * Scale;
	int columns = 9;
	int margin = 2 * Scale;
	int box_w = columns * cell_w + 2 * margin;
	int box_h = rows * cell_h + 2 * margin;
	int2 size = int2(BUFFER_WIDTH, BUFFER_HEIGHT);
	int2 origin = int2((Corner == 1 || Corner == 3) ? size.x - box_w - margin : margin, (Corner >= 2) ? size.y - box_h - margin : margin);
	int2 p = int2(position.xy) - origin;
	if (p.x < 0 || p.y < 0 || p.x >= box_w || p.y >= box_h)
		return color;

	float3 result = color.rgb * 0.35;
	int2 t = p - int2(margin, margin);
	if (t.x >= 0 && t.y >= 0 && t.x < columns * cell_w && t.y < rows * cell_h)
	{
		int row = t.y / cell_h;
		int line = Details ? row : (row == 0 ? 0 : 5);
		int col = t.x / cell_w;
		int2 pixel = int2((t.x % cell_w) / Scale, (t.y % cell_h) / Scale);
		if (pixel.x < 3 && pixel.y < 5)
		{
			int g = CharAt(line, col, values[line]);
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
