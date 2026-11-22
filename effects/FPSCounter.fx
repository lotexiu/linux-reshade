// Desempenho no canto da tela (o equivalente ao OSD de FPS do ReShade), escrito para este projeto.
//  - FPS agora (suavizado), média, mínimo, máximo e 1% low dos últimos 5 s e o tempo de quadro, com cor pela fluidez (verde, amarelo, vermelho);
//  - um gráfico dos últimos tempos de quadro (a linha clara é a de 60 FPS);
//  - um ícone de "processando" (giratório) quando o programa compila ou refaz efeitos para o jogo.
// Usa as fontes do runtime `frametime`, `fps_*`, `lr_busy` e `timer`. Fonte de 5x7 pixels, com o mesmo tamanho de qualquer resolução (Tamanho).

uniform int Corner <
	ui_type = "combo";
	ui_label = "Canto";
	ui_items = "Superior esquerdo\0Superior direito\0Inferior esquerdo\0Inferior direito\0";
> = 0;

uniform int Scale <
	ui_type = "slider";
	ui_label = "Tamanho";
	ui_min = 1; ui_max = 8;
> = 2;

uniform float Smoothing <
	ui_type = "slider";
	ui_label = "Suavização";
	ui_tooltip = "Quanto cada quadro novo pesa no FPS \"agora\" (menor = mais estável).";
	ui_min = 0.005; ui_max = 1.0;
> = 0.05;

uniform bool ShowStats <
	ui_label = "Mostrar o contador";
> = true;

uniform bool Details <
	ui_label = "Média, mínimo, máximo e 1% low";
	ui_tooltip = "Desligado mostra só o FPS e o tempo de quadro.";
> = true;

uniform bool ShowBusy <
	ui_label = "Ícone de processando";
	ui_tooltip = "Um indicador girando enquanto efeitos são compilados ou refeitos para este jogo.";
> = true;

uniform float FRAME_TIME < source = "frametime"; >;
uniform float FPS_AVG < source = "fps_avg"; >;
uniform float FPS_MIN < source = "fps_min"; >;
uniform float FPS_MAX < source = "fps_max"; >;
uniform float FPS_LOW1 < source = "fps_low1"; >;
uniform float BUSY < source = "lr_busy"; >;
uniform float TIMER < source = "timer"; >;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

#define HISTORY 96

// A média do tempo de quadro, em duas texturas de um pixel que se revezam (um passe não lê o que escreve).
texture AvgA { Width = 1; Height = 1; Format = R32F; };
texture AvgB { Width = 1; Height = 1; Format = R32F; };
sampler sAvgA { Texture = AvgA; MagFilter = POINT; MinFilter = POINT; };
sampler sAvgB { Texture = AvgB; MagFilter = POINT; MinFilter = POINT; };

// Os últimos tempos de quadro (o mais novo à direita), também em duas texturas que se revezam.
texture HistA { Width = HISTORY; Height = 1; Format = R32F; };
texture HistB { Width = HISTORY; Height = 1; Format = R32F; };
sampler sHistA { Texture = HistA; MagFilter = POINT; MinFilter = POINT; };
sampler sHistB { Texture = HistB; MagFilter = POINT; MinFilter = POINT; };

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

float PS_HistUpdate(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	int x = (int)position.x;
	return x >= HISTORY - 1 ? FRAME_TIME : tex2Dfetch(sHistA, int2(x + 1, 0)).x;
}

float PS_HistCopy(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2Dfetch(sHistB, int2((int)position.x, 0)).x;
}

// Fonte de 5x7: 7 linhas de 5 bits por caractere (bit 4 é a coluna da esquerda).
// 0-9 dígitos, 10 '.', 11 F, 12 P, 13 S, 14 M, 15 espaço, 16 A, 17 V, 18 G, 19 I, 20 N, 21 X, 22 L, 23 %.
uint GlyphRow(int g, int y)
{
	uint font[168] = {
		0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E, 0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E,
		0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F, 0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E,
		0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02, 0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E,
		0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E, 0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08,
		0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E, 0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C,
		0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10,
		0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10, 0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E,
		0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
		0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04,
		0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E, 0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E,
		0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11,
		0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F, 0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13
	};
	return font[clamp(g, 0, 23) * 7 + clamp(y, 0, 6)];
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
// (5) mostra `value` como décimos de milissegundo ("12.3").
int CharAt(int line, int i, int value)
{
	if (i < 3) return Label(line, i);
	if (i == 3) return 15;
	int k = i - 4;
	value = clamp(value, 0, 99999);
	if (line == 5)
	{
		int digits = value >= 1000 ? 4 : (value >= 100 ? 3 : 2);
		int lead = 4 - digits;
		if (k == 3) return 10;
		int pos = k < 3 ? k : k - 1;
		if (pos < lead) return 15;
		return (value / Pow10(3 - pos)) % 10;
	}
	int n = value >= 10000 ? 5 : (value >= 1000 ? 4 : (value >= 100 ? 3 : (value >= 10 ? 2 : 1)));
	if (k >= n) return 15;
	return (value / Pow10(n - 1 - k)) % 10;
}

// A cor pela fluidez: verde a partir de 55 FPS, amarelo a partir de 30, vermelho abaixo.
float3 FpsColor(float fps)
{
	return fps >= 55.0 ? float3(0.40, 1.00, 0.50) : (fps >= 30.0 ? float3(1.00, 0.85, 0.30) : float3(1.00, 0.38, 0.32));
}

// Distância (em pixels) a um retângulo de cantos arredondados: negativa dentro.
float RoundedBox(float2 p, float2 size, float radius)
{
	float2 q = abs(p - size * 0.5) - (size * 0.5 - radius);
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float4 PS_Draw(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	bool busy = ShowBusy && BUSY > 0.5;
	if (!ShowStats && !busy)
		return color;

	float frame_ms = tex2Dfetch(sAvgB, int2(0, 0)).x;
	float fps_now = 1000.0 / max(frame_ms, 0.01);
	int values[6];
	values[0] = (int)clamp(round(fps_now), 0.0, 9999.0);
	values[1] = (int)clamp(round(FPS_AVG), 0.0, 9999.0);
	values[2] = (int)clamp(round(FPS_MIN), 0.0, 9999.0);
	values[3] = (int)clamp(round(FPS_MAX), 0.0, 9999.0);
	values[4] = (int)clamp(round(FPS_LOW1), 0.0, 9999.0);
	values[5] = (int)clamp(round(frame_ms * 10.0), 0.0, 9999.0);

	int s = Scale;
	int cw = 6 * s;
	int ch = 8 * s;
	int pad = 3 * s;
	int margin = 4 * s;
	int spinner = 10 * s;
	int rows = ShowStats ? (Details ? 6 : 2) : 0;
	int text_w = 9 * cw;
	int graph_h = ShowStats ? 12 * s : 0;
	int graph_gap = ShowStats ? 2 * s : 0;
	int box_w = ShowStats ? text_w + 2 * pad : spinner + 2 * pad;
	int box_h = ShowStats ? rows * ch + graph_gap + graph_h + 2 * pad : spinner + 2 * pad;
	int2 size = int2(BUFFER_WIDTH, BUFFER_HEIGHT);
	int2 origin = int2((Corner == 1 || Corner == 3) ? size.x - box_w - margin : margin, (Corner >= 2) ? size.y - box_h - margin : margin);
	int2 p = int2(position.xy) - origin;
	float edge = RoundedBox(float2(p), float2(box_w, box_h), 3.0 * s);
	if (edge > 0.5)
		return color;

	// Fundo: escurece o que está atrás (o jogo continua visível) e uma borda clara e fina.
	float3 result = lerp(color.rgb, float3(0.02, 0.02, 0.03), 0.68);
	if (edge > -1.0)
		result = lerp(result, float3(1.0, 1.0, 1.0), 0.18);

	int2 t = p - int2(pad, pad);
	if (ShowStats && t.x >= 0 && t.y >= 0 && t.x < text_w)
	{
		if (t.y < rows * ch)
		{
			int row = t.y / ch;
			int line = Details ? row : (row == 0 ? 0 : 5);
			int col = t.x / cw;
			int px = (t.x % cw) / s;
			int py = (t.y % ch) / s;
			if (px < 5 && py < 7)
			{
				int g = CharAt(line, col, values[line]);
				uint bits = GlyphRow(g, py);
				if (((bits >> (4u - (uint)px)) & 1u) != 0u)
				{
					float3 ink = float3(1.0, 1.0, 1.0);
					if (col < 3)
						ink = float3(0.66, 0.70, 0.78);
					else if (line == 5)
						ink = FpsColor(1000.0 / max(frame_ms, 0.01));
					else
						ink = FpsColor((float)values[line]);
					result = ink;
				}
			}
		}
		else if (t.y >= rows * ch + graph_gap && t.y < rows * ch + graph_gap + graph_h)
		{
			// O gráfico dos últimos tempos de quadro: cada coluna uma barra de 0 a 50 ms; a linha clara marca 16,7 ms (60 FPS).
			int gx = clamp(t.x * HISTORY / text_w, 0, HISTORY - 1);
			float ms = tex2Dfetch(sHistA, int2(gx, 0)).x;
			float height = clamp(ms / 50.0, 0.0, 1.0) * graph_h;
			float from_bottom = (float)(rows * ch + graph_gap + graph_h - t.y);
			if (from_bottom <= height)
				result = lerp(result, FpsColor(1000.0 / max(ms, 0.01)), 0.85);
			if (abs(from_bottom - (16.7 / 50.0) * graph_h) < 0.6)
				result = lerp(result, float3(1.0, 1.0, 1.0), 0.35);
		}
	}

	if (busy)
	{
		// O ícone de processando: um arco que gira, com a cauda apagando. No canto direito de cima do painel, ou sozinho no painel pequeno.
		float2 center = ShowStats ? float2(box_w - pad - spinner * 0.5, pad + spinner * 0.5) : float2(box_w * 0.5, box_h * 0.5);
		float2 d = float2(p) - center;
		float r = length(d);
		if (r > spinner * 0.30 && r < spinner * 0.50)
		{
			float turn = frac(atan2(d.y, d.x) / 6.2831853 - TIMER * 0.0012);
			float tail = turn < 0.7 ? turn / 0.7 : 0.0;
			result = lerp(result, float3(0.45, 0.78, 1.00), tail);
		}
	}
	return float4(result, color.a);
}

technique FPSCounter < ui_label = "Contador de FPS"; >
{
	pass Update { VertexShader = VS; PixelShader = PS_Update; RenderTarget = AvgB; }
	pass Copy { VertexShader = VS; PixelShader = PS_Copy; RenderTarget = AvgA; }
	pass HistoryUpdate { VertexShader = VS; PixelShader = PS_HistUpdate; RenderTarget = HistB; }
	pass HistoryCopy { VertexShader = VS; PixelShader = PS_HistCopy; RenderTarget = HistA; }
	pass Draw { VertexShader = VS; PixelShader = PS_Draw; }
}
