/**
 * Deluge.Toolbar.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

/**
 * An extension of the <tt>Ext.Toolbar</tt> class that provides an extensible toolbar for Deluge.
 * @class Deluge.Toolbar
 * @extends Ext.Toolbar
 */
Deluge.Toolbar = Ext.extend(Ext.Toolbar, {
    constructor: function (config) {
        config = Ext.apply(
            {
                items: [
                    {
                        id: 'tbar-deluge-text',
                        // The fork's own name. It stays "Deluge" everywhere a
                        // client reads it, the version the daemon reports and
                        // the API included, because that is what clients
                        // written against the Python server expect. This is
                        // the one place a person reads it.
                        text: _('RE:deluge'),
                        iconCls: 'x-deluge-main-panel',
                        handler: this.onAboutClick,
                    },
                    new Ext.Toolbar.Separator(),
                    {
                        id: 'create',
                        disabled: true,
                        hidden: true,
                        text: _('Create'),
                        iconCls: 'icon-create',
                        handler: this.onTorrentAction,
                    },
                    {
                        id: 'add',
                        disabled: true,
                        text: _('Add'),
                        iconCls: 'icon-add',
                        handler: this.onTorrentAdd,
                    },
                    {
                        id: 'remove',
                        disabled: true,
                        text: _('Remove'),
                        iconCls: 'icon-remove',
                        handler: this.onTorrentAction,
                    },
                    new Ext.Toolbar.Separator(),
                    {
                        id: 'pause',
                        disabled: true,
                        text: _('Pause'),
                        iconCls: 'icon-pause',
                        handler: this.onTorrentAction,
                    },
                    {
                        id: 'resume',
                        disabled: true,
                        text: _('Resume'),
                        iconCls: 'icon-resume',
                        handler: this.onTorrentAction,
                    },
                    new Ext.Toolbar.Separator(),
                    {
                        id: 'up',
                        cls: 'x-btn-text-icon',
                        disabled: true,
                        text: _('Up'),
                        iconCls: 'icon-up',
                        handler: this.onTorrentAction,
                    },
                    {
                        id: 'down',
                        disabled: true,
                        text: _('Down'),
                        iconCls: 'icon-down',
                        handler: this.onTorrentAction,
                    },
                    new Ext.Toolbar.Separator(),
                    {
                        id: 'preferences',
                        text: _('Preferences'),
                        iconCls: 'x-deluge-preferences',
                        handler: this.onPreferencesClick,
                        scope: this,
                    },
                    {
                        // What the daemon did on its own. Beside Preferences,
                        // which is where the rules that did it are turned on,
                        // and enabled only once there is a daemon to ask.
                        id: 'activity',
                        disabled: true,
                        text: _('Activity'),
                        iconCls: 'icon-ok',
                        handler: this.onActivityClick,
                        scope: this,
                    },
                    {
                        id: 'connectionman',
                        text: _('Connection Manager'),
                        iconCls: 'x-deluge-connection-manager',
                        handler: this.onConnectionManagerClick,
                        scope: this,
                    },
                    '->',
                    '->',
                    {
                        // The daemon's `keyword` filter searches the name, the
                        // state, the tracker and its last message, the label
                        // and the infohash, and every term has to match. All
                        // of that existed and nothing ever sent it.
                        id: 'search',
                        xtype: 'textfield',
                        width: 170,
                        emptyText: _('Search'),
                        enableKeyEvents: true,
                        listeners: {
                            keyup: {
                                fn: this.onSearchKey,
                                scope: this,
                                // Buffered: a poll per keystroke would be one
                                // request per letter typed.
                                buffer: 400,
                            },
                            specialkey: {
                                fn: this.onSearchSpecialKey,
                                scope: this,
                            },
                        },
                    },
                    {
                        id: 'help',
                        iconCls: 'icon-help',
                        text: _('Help'),
                        handler: this.onHelpClick,
                        scope: this,
                    },
                    {
                        id: 'logout',
                        iconCls: 'icon-logout',
                        disabled: true,
                        text: _('Logout'),
                        handler: this.onLogout,
                        scope: this,
                    },
                ],
            },
            config
        );
        Deluge.Toolbar.superclass.constructor.call(this, config);
    },

    connectedButtons: [
        'add',
        'remove',
        'pause',
        'resume',
        'up',
        'down',
        'activity',
    ],

    initComponent: function () {
        Deluge.Toolbar.superclass.initComponent.call(this);
        deluge.events.on('connect', this.onConnect, this);
        deluge.events.on('login', this.onLogin, this);
    },

    onConnect: function () {
        Ext.each(
            this.connectedButtons,
            function (buttonId) {
                this.items.get(buttonId).enable();
            },
            this
        );
    },

    onDisconnect: function () {
        Ext.each(
            this.connectedButtons,
            function (buttonId) {
                this.items.get(buttonId).disable();
            },
            this
        );
    },

    onLogin: function () {
        this.items.get('logout').enable();
    },

    onLogout: function () {
        this.items.get('logout').disable();
        deluge.login.logout();
    },

    onConnectionManagerClick: function () {
        deluge.connectionManager.show();
    },

    /**
     * What is typed in the search box, trimmed, or nothing.
     */
    getSearch: function () {
        var field = this.items.get('search');
        if (!field) return '';
        return (field.getValue() || '').trim();
    },

    onSearchKey: function () {
        // The poll carries the term, so asking for one now is the whole of it.
        deluge.ui.update();
    },

    onSearchSpecialKey: function (field, e) {
        if (e.getKey() === e.ESC) {
            field.setValue('');
            deluge.ui.update();
        }
    },

    onActivityClick: function () {
        // Built when it is first wanted rather than with the interface: most
        // sessions never open it.
        if (!deluge.activityWindow) {
            deluge.activityWindow = new Deluge.ActivityWindow();
        }
        deluge.activityWindow.show();
    },

    onHelpClick: function () {
        window.open('https://github.com/Sn0wAlice/redeluge/wiki', '_blank');
    },

    onAboutClick: function () {
        var about = new Deluge.about.AboutWindow();
        about.show();
    },

    onPreferencesClick: function () {
        deluge.preferences.show();
    },

    onTorrentAction: function (item) {
        var selection = deluge.torrents.getSelections();
        var ids = [];
        Ext.each(selection, function (record) {
            ids.push(record.id);
        });

        switch (item.id) {
            case 'remove':
                deluge.removeWindow.show(ids);
                break;
            case 'pause':
            case 'resume':
                deluge.client.core[item.id + '_torrent'](ids, {
                    success: function () {
                        deluge.ui.update();
                    },
                });
                break;
            case 'up':
            case 'down':
                deluge.client.core['queue_' + item.id](ids, {
                    success: function () {
                        deluge.ui.update();
                    },
                });
                break;
        }
    },

    onTorrentAdd: function () {
        deluge.add.show();
    },
});
