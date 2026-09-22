/**
 * Deluge.AboutWindow.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

Ext.namespace('Deluge.about');

/**
 * @class Deluge.about.AboutWindow
 * @extends Ext.Window
 */
Deluge.about.AboutWindow = Ext.extend(Ext.Window, {
    id: 'AboutWindow',
    title: _('About RE:deluge'),
    height: 350,
    width: 290,
    iconCls: 'x-deluge-main-panel',
    resizable: false,
    plain: true,
    layout: {
        type: 'vbox',
        align: 'center',
    },
    buttonAlign: 'center',

    initComponent: function () {
        Deluge.about.AboutWindow.superclass.initComponent.call(this);
        this.addEvents({
            build_ready: true,
        });

        var self = this;
        var libtorrent = function () {
            deluge.client.core.get_libtorrent_version({
                success: function (lt_version) {
                    comment += '<br/>' + _('libtorrent:') + ' ' + lt_version;
                    Ext.getCmp('about_comment').setText(comment, false);
                    self.fireEvent('build_ready');
                },
            });
        };

        // Two different versions, deliberately. `deluge.version` is what this
        // server reports to clients, which is Deluge's, because a client
        // written against the Python server expects to be talking to it.
        // `redeluge_version` is the fork's own, and this window is the only
        // place a person is shown it.
        var client_version = deluge.version;
        var our_version = deluge.config.redeluge_version || '';

        var comment =
            _(
                'A peer-to-peer file sharing program\nutilizing the BitTorrent protocol.'
            ).replace('\n', '<br/>') +
            '<br/><br/>' +
            _('A fork of Deluge, rewritten in Rust.') +
            '<br/><br/>' +
            _('Reports to clients as Deluge:') +
            ' ' +
            client_version +
            '<br/>';
        deluge.client.web.connected({
            success: function (connected) {
                if (connected) {
                    deluge.client.daemon.get_version({
                        success: function (server_version) {
                            comment +=
                                _('Server:') + ' ' + server_version + '<br/>';
                            libtorrent();
                        },
                    });
                } else {
                    this.fireEvent('build_ready');
                }
            },
            failure: function () {
                this.fireEvent('build_ready');
            },
            scope: this,
        });

        this.add([
            {
                xtype: 'box',
                style: 'padding-top: 5px',
                height: 80,
                width: 240,
                cls: 'x-deluge-logo',
                hideLabel: true,
            },
            {
                xtype: 'label',
                style: 'padding-top: 10px; font-weight: bold; font-size: 16px;',
                text: our_version
                    ? _('RE:deluge') + ' ' + our_version
                    : _('RE:deluge'),
            },
            {
                xtype: 'label',
                id: 'about_comment',
                style: 'padding-top: 10px; text-align:center; font-size: 12px;',
                html: comment,
            },
            {
                xtype: 'label',
                style: 'padding-top: 10px; font-size: 10px; text-align: center;',
                html:
                    _('Copyright 2007-2025 Deluge Team') +
                    '<br/>' +
                    _('RE:deluge, GPL-3.0-or-later'),
            },
            {
                xtype: 'label',
                style: 'padding-top: 5px; font-size: 12px;',
                html: '<a href="https://github.com/retorrent/redeluge" target="_blank" rel="noopener">github.com/retorrent/redeluge</a>',
            },
        ]);
        this.addButton(_('Close'), this.onCloseClick, this);
    },

    show: function () {
        this.on('build_ready', function () {
            Deluge.about.AboutWindow.superclass.show.call(this);
        });
    },

    onCloseClick: function () {
        this.close();
    },
});

Ext.namespace('Deluge');

Deluge.About = function () {
    new Deluge.about.AboutWindow().show();
};
