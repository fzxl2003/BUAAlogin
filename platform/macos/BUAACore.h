#ifndef BUAA_CORE_H
#define BUAA_CORE_H
#include <stdint.h>
typedef struct BuaaClient BuaaClient;
typedef void (*BuaaCallback)(void *context, const char *event_json);
uint32_t buaa_abi_version(void);
BuaaClient *buaa_create(const char *config_json, BuaaCallback callback, void *context);
int32_t buaa_start(BuaaClient *client);
int32_t buaa_scan(BuaaClient *client);
void buaa_stop(BuaaClient *client);
void buaa_network_changed(BuaaClient *client);
void buaa_network_changed_interface(BuaaClient *client, const char *name);
void buaa_suspend(BuaaClient *client);
void buaa_resume(BuaaClient *client);
void buaa_tick(BuaaClient *client);
void buaa_free(BuaaClient *client);
#endif
