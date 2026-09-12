/**
 * Deluge.Menus.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

deluge.menus = {
    onTorrentActionSetOpt: function (item, e) {
        var ids = deluge.torrents.getSelectedIds();
        var action = item.initialConfig.torrentAction;
        var opts = {};
        opts[action[0]] = action[1];
        deluge.client.core.set_torrent_options(ids, opts);
    },

    onTorrentActionMethod: function (item, e) {
        var ids = deluge.torrents.getSelectedIds();
        var action = item.initialConfig.torrentAction;
        deluge.client.core[action](ids, {
            success: function () {
                deluge.ui.update();
            },
        });
    },

    /**
     * Puts the selected torrents in a label, or takes them out of one.
     *
     * `label.set_torrent` rather than `core.set_torrent_options`, because it is
     * the Label plugin's own call: it creates the label if it has gone missing
     * and applies whatever rules the label carries, which setting the option
     * directly would skip.
     */
    onLabelPicked: function (item) {
        var ids = deluge.torrents.getSelectedIds();
        if (!ids.length) return;

        var label = item.initialConfig.labelName || '';
        var left = ids.length;
        Ext.each(ids, function (id) {
            deluge.client.label.set_torrent(id, label, {
                success: function () {
                    if (--left === 0) deluge.ui.update();
                },
                failure: function () {
                    if (--left === 0) deluge.ui.update();
                },
            });
        });
    },

    /**
     * Rebuilds the Label submenu from the labels that exist.
     *
     * Read every time the menu opens rather than cached: another program adds
     * labels through the same API, and a list that is one page-load old is the
     * list that does not have the one you just made.
     */
    refreshLabelMenu: function () {
        var menu = deluge.menus.label;
        if (!menu) return;

        deluge.client.label.get_labels({
            success: function (labels) {
                menu.removeAll(true);

                // Only marked when one torrent is selected: with several, the
                // menu sets them all and there is no single current label to
                // show as chosen.
                var current = null;
                var records = deluge.torrents.getSelections() || [];
                if (records.length === 1) {
                    current = records[0].get('label') || '';
                }

                menu.add({
                    text: _('No Label'),
                    labelName: '',
                    checked: current === '',
                    group: 'torrent-label',
                    handler: deluge.menus.onLabelPicked,
                    scope: deluge.menus,
                });

                if (labels && labels.length) {
                    menu.add('-');
                    Ext.each(labels, function (name) {
                        menu.add({
                            text: name,
                            labelName: name,
                            checked: current === name,
                            group: 'torrent-label',
                            handler: deluge.menus.onLabelPicked,
                            scope: deluge.menus,
                        });
                    });
                } else {
                    menu.add('-');
                    menu.add({
                        text: _('No labels yet, add one in Preferences'),
                        disabled: true,
                    });
                }
            },
            failure: function () {
                menu.removeAll(true);
                menu.add({ text: _('Labels are unavailable'), disabled: true });
            },
        });
    },

    onTorrentActionShow: function (item, e) {
        var ids = deluge.torrents.getSelectedIds();
        var action = item.initialConfig.torrentAction;
        switch (action) {
            case 'copy_magnet':
                deluge.copyMagnetWindow.show();
                break;
            case 'edit_trackers':
                deluge.editTrackers.show();
                break;
            case 'remove':
                deluge.removeWindow.show(ids);
                break;
            case 'move':
                deluge.moveStorage.show(ids);
                break;
        }
    },
};

/**
 * The labels a torrent can be put in, filled in when the menu opens.
 */
deluge.menus.label = new Ext.menu.Menu({
    id: 'torrentLabelMenu',
    items: [],
});

deluge.menus.torrent = new Ext.menu.Menu({
    id: 'torrentMenu',
    items: [
        {
            torrentAction: 'pause_torrent',
            text: _('Pause'),
            iconCls: 'icon-pause',
            handler: deluge.menus.onTorrentActionMethod,
            scope: deluge.menus,
        },
        {
            torrentAction: 'resume_torrent',
            text: _('Resume'),
            iconCls: 'icon-resume',
            handler: deluge.menus.onTorrentActionMethod,
            scope: deluge.menus,
        },
        '-',
        {
            text: _('Options'),
            iconCls: 'icon-options',
            hideOnClick: false,
            menu: new Ext.menu.Menu({
                items: [
                    {
                        text: _('D/L Speed Limit'),
                        iconCls: 'x-deluge-downloading',
                        hideOnClick: false,
                        menu: new Ext.menu.Menu({
                            items: [
                                {
                                    torrentAction: ['max_download_speed', 5],
                                    text: _('5 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_download_speed', 10],
                                    text: _('10 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_download_speed', 30],
                                    text: _('30 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_download_speed', 80],
                                    text: _('80 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_download_speed', 300],
                                    text: _('300 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_download_speed', -1],
                                    text: _('Unlimited'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                            ],
                        }),
                    },
                    {
                        text: _('U/L Speed Limit'),
                        iconCls: 'x-deluge-seeding',
                        hideOnClick: false,
                        menu: new Ext.menu.Menu({
                            items: [
                                {
                                    torrentAction: ['max_upload_speed', 5],
                                    text: _('5 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_speed', 10],
                                    text: _('10 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_speed', 30],
                                    text: _('30 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_speed', 80],
                                    text: _('80 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_speed', 300],
                                    text: _('300 KiB/s'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_speed', -1],
                                    text: _('Unlimited'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                            ],
                        }),
                    },
                    {
                        text: _('Connection Limit'),
                        iconCls: 'x-deluge-connections',
                        hideOnClick: false,
                        menu: new Ext.menu.Menu({
                            items: [
                                {
                                    torrentAction: ['max_connections', 50],
                                    text: '50',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_connections', 100],
                                    text: '100',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_connections', 200],
                                    text: '200',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_connections', 300],
                                    text: '300',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_connections', 500],
                                    text: '500',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_connections', -1],
                                    text: _('Unlimited'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                            ],
                        }),
                    },
                    {
                        text: _('Upload Slot Limit'),
                        iconCls: 'icon-upload-slots',
                        hideOnClick: false,
                        menu: new Ext.menu.Menu({
                            items: [
                                {
                                    torrentAction: ['max_upload_slots', 0],
                                    text: '0',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_slots', 1],
                                    text: '1',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_slots', 2],
                                    text: '2',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_slots', 3],
                                    text: '3',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_slots', 5],
                                    text: '5',
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['max_upload_slots', -1],
                                    text: _('Unlimited'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                            ],
                        }),
                    },
                    {
                        id: 'auto_managed',
                        text: _('Auto Managed'),
                        hideOnClick: false,
                        menu: new Ext.menu.Menu({
                            items: [
                                {
                                    torrentAction: ['auto_managed', true],
                                    text: _('On'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                                {
                                    torrentAction: ['auto_managed', false],
                                    text: _('Off'),
                                    handler: deluge.menus.onTorrentActionSetOpt,
                                    scope: deluge.menus,
                                },
                            ],
                        }),
                    },
                ],
            }),
        },
        '-',
        {
            text: _('Queue'),
            iconCls: 'icon-queue',
            hideOnClick: false,
            menu: new Ext.menu.Menu({
                items: [
                    {
                        torrentAction: 'queue_top',
                        text: _('Top'),
                        iconCls: 'icon-top',
                        handler: deluge.menus.onTorrentActionMethod,
                        scope: deluge.menus,
                    },
                    {
                        torrentAction: 'queue_up',
                        text: _('Up'),
                        iconCls: 'icon-up',
                        handler: deluge.menus.onTorrentActionMethod,
                        scope: deluge.menus,
                    },
                    {
                        torrentAction: 'queue_down',
                        text: _('Down'),
                        iconCls: 'icon-down',
                        handler: deluge.menus.onTorrentActionMethod,
                        scope: deluge.menus,
                    },
                    {
                        torrentAction: 'queue_bottom',
                        text: _('Bottom'),
                        iconCls: 'icon-bottom',
                        handler: deluge.menus.onTorrentActionMethod,
                        scope: deluge.menus,
                    },
                ],
            }),
        },
        '-',
        {
            torrentAction: 'copy_magnet',
            text: _('Copy Magnet URI'),
            iconCls: 'icon-magnet-copy',
            handler: deluge.menus.onTorrentActionShow,
            scope: deluge.menus,
        },
        {
            torrentAction: 'force_reannounce',
            text: _('Update Tracker'),
            iconCls: 'icon-update-tracker',
            handler: deluge.menus.onTorrentActionMethod,
            scope: deluge.menus,
        },
        {
            torrentAction: 'edit_trackers',
            text: _('Edit Trackers'),
            iconCls: 'icon-edit-trackers',
            handler: deluge.menus.onTorrentActionShow,
            scope: deluge.menus,
        },
        '-',
        {
            torrentAction: 'remove',
            text: _('Remove Torrent'),
            iconCls: 'icon-remove',
            handler: deluge.menus.onTorrentActionShow,
            scope: deluge.menus,
        },
        '-',
        {
            torrentAction: 'force_recheck',
            text: _('Force Recheck'),
            iconCls: 'icon-recheck',
            handler: deluge.menus.onTorrentActionMethod,
            scope: deluge.menus,
        },
        {
            torrentAction: 'move',
            text: _('Move Download Folder'),
            iconCls: 'icon-move',
            handler: deluge.menus.onTorrentActionShow,
            scope: deluge.menus,
        },
        {
            text: _('Label'),
            iconCls: 'icon-label',
            menu: deluge.menus.label,
        },
    ],
});

deluge.menus.filePriorities = new Ext.menu.Menu({
    id: 'filePrioritiesMenu',
    items: [
        {
            id: 'expandAll',
            text: _('Expand All'),
            iconCls: 'icon-expand-all',
        },
        '-',
        {
            id: 'skip',
            text: _('Skip'),
            iconCls: 'icon-do-not-download',
            filePriority: FILE_PRIORITY['Skip'],
        },
        {
            id: 'low',
            text: _('Low'),
            iconCls: 'icon-low',
            filePriority: FILE_PRIORITY['Low'],
        },
        {
            id: 'normal',
            text: _('Normal'),
            iconCls: 'icon-normal',
            filePriority: FILE_PRIORITY['Normal'],
        },
        {
            id: 'high',
            text: _('High'),
            iconCls: 'icon-high',
            filePriority: FILE_PRIORITY['High'],
        },
    ],
});
