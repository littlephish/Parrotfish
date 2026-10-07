#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <speex/speex.h>

#define MAX_FRAME 640
#define MAX_PACKET 4000
#define LOST 0xFFFF

static const SpeexMode *mode_for(const char *name)
{
    if (!strcmp(name, "nb")) return speex_lib_get_mode(SPEEX_MODEID_NB);
    if (!strcmp(name, "wb")) return speex_lib_get_mode(SPEEX_MODEID_WB);
    if (!strcmp(name, "uwb")) return speex_lib_get_mode(SPEEX_MODEID_UWB);
    fprintf(stderr, "band must be nb, wb or uwb\n");
    exit(2);
}

static int read_record(FILE *in, unsigned char *buf, int *lost)
{
    unsigned char head[2];
    int length;
    if (fread(head, 1, 2, in) != 2) return -1;
    length = head[0] | (head[1] << 8);
    *lost = length == LOST;
    if (*lost) return 0;
    if (length > MAX_PACKET || fread(buf, 1, length, in) != (size_t)length) return -1;
    return length;
}

static int encode(int argc, char **argv)
{
    const SpeexMode *mode = mode_for(argv[2]);
    int quality = atoi(argv[3]);
    int vbr = atoi(argv[4]);
    int per_packet = atoi(argv[5]);
    FILE *in = fopen(argv[6], "rb");
    FILE *out = fopen(argv[7], "wb");
    void *st = speex_encoder_init(mode);
    SpeexBits bits;
    int frame_size, i, k, done = 0, packets = 0;
    short pcm[MAX_FRAME];
    float input[MAX_FRAME];
    unsigned char buf[MAX_PACKET];
    (void)argc;
    if (!in || !out) { fprintf(stderr, "cannot open files\n"); return 2; }
    speex_encoder_ctl(st, SPEEX_GET_FRAME_SIZE, &frame_size);
    speex_encoder_ctl(st, SPEEX_SET_QUALITY, &quality);
    if (vbr) {
        float vbr_quality = (float)quality;
        speex_encoder_ctl(st, SPEEX_SET_VBR, &vbr);
        speex_encoder_ctl(st, SPEEX_SET_VBR_QUALITY, &vbr_quality);
    }
    speex_bits_init(&bits);
    while (!done) {
        int n;
        speex_bits_reset(&bits);
        for (k = 0; k < per_packet; k++) {
            if (fread(pcm, sizeof(short), frame_size, in) != (size_t)frame_size) { done = 1; break; }
            for (i = 0; i < frame_size; i++) input[i] = pcm[i];
            speex_encode(st, input, &bits);
        }
        if (k == 0) break;
        n = speex_bits_write(&bits, (char *)buf, MAX_PACKET);
        fputc(n & 0xFF, out);
        fputc(n >> 8, out);
        fwrite(buf, 1, n, out);
        packets++;
    }
    fprintf(stderr, "%s q%d vbr%d x%d: %d packets, frame %d samples\n", argv[2], quality, vbr, per_packet, packets, frame_size);
    return 0;
}

static int decode(int argc, char **argv)
{
    const SpeexMode *mode = mode_for(argv[2]);
    int enhance = atoi(argv[3]);
    FILE *in = fopen(argv[4], "rb");
    FILE *out = fopen(argv[5], "wb");
    void *st = speex_decoder_init(mode);
    SpeexBits bits;
    int frame_size, length, lost, frames_last = 1, total = 0, packets = 0, k;
    float output[MAX_FRAME];
    unsigned char buf[MAX_PACKET];
    (void)argc;
    if (!in || !out) { fprintf(stderr, "cannot open files\n"); return 2; }
    speex_decoder_ctl(st, SPEEX_GET_FRAME_SIZE, &frame_size);
    speex_decoder_ctl(st, SPEEX_SET_ENH, &enhance);
    speex_bits_init(&bits);
    while ((length = read_record(in, buf, &lost)) >= 0) {
        packets++;
        if (lost) {
            for (k = 0; k < frames_last; k++) {
                speex_decode(st, NULL, output);
                fwrite(output, sizeof(float), frame_size, out);
                total++;
            }
            continue;
        }
        speex_bits_read_from(&bits, (char *)buf, length);
        for (k = 0; k < 10; k++) {
            int ret = speex_decode(st, &bits, output);
            if (ret != 0) break;
            if (speex_bits_remaining(&bits) < 0) break;
            fwrite(output, sizeof(float), frame_size, out);
            total++;
        }
        if (k > 0) frames_last = k;
    }
    fprintf(stderr, "%s: %d packets, %d frames of %d samples\n", argv[2], packets, total, frame_size);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc == 8 && !strcmp(argv[1], "enc")) return encode(argc, argv);
    if (argc == 6 && !strcmp(argv[1], "dec")) return decode(argc, argv);
    fprintf(stderr, "usage: speexref enc <nb|wb|uwb> <quality> <vbr 0|1> <frames per packet> <in.s16> <out.pkt>\n"
                    "       speexref dec <nb|wb|uwb> <enhance 0|1> <in.pkt> <out.f32>\n");
    return 2;
}
